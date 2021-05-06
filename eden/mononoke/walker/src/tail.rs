/*
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * This software may be used and distributed according to the terms of the
 * GNU General Public License version 2.
 */

use crate::checkpoint::{Checkpoint, CheckpointsByName};
use crate::graph::{ChangesetKey, Node, NodeType};
use crate::log;
use crate::setup::JobWalkParams;
use crate::state::InternedType;
use crate::walk::{
    walk_exact, OutgoingEdge, RepoWalkParams, RepoWalkTypeParams, StepRoute, TailingWalkVisitor,
    WalkVisitor,
};

use anyhow::{anyhow, bail, Error};
use bonsai_hg_mapping::BonsaiOrHgChangesetIds;
use bulkops::{Direction, PublicChangesetBulkFetch, MAX_FETCH_STEP};
use cloned::cloned;
use context::CoreContext;
use derived_data::BonsaiDerivable;
use derived_data_filenodes::FilenodesOnlyPublic;
use fbinit::FacebookInit;
use futures::{
    future::{self, Future},
    stream::{self, BoxStream, StreamExt, TryStreamExt},
};
use mercurial_derived_data::MappedHgChangesetId;
use mononoke_types::{ChangesetId, RepositoryId, Timestamp};
use slog::{info, Logger};
use std::{
    cmp::{max, min},
    collections::HashSet,
    sync::Arc,
};
use strum::IntoEnumIterator;
use tokio::time::{Duration, Instant};

// We can chose to go direct from the ChangesetId to types keyed by it without loading the Changeset
fn roots_for_chunk(
    ids: HashSet<ChangesetId>,
    root_types: &HashSet<NodeType>,
) -> Result<Vec<OutgoingEdge>, Error> {
    let mut edges = vec![];
    for id in ids {
        for r in root_types {
            let n = match r {
                NodeType::BonsaiHgMapping => Node::BonsaiHgMapping(ChangesetKey {
                    inner: id,
                    filenode_known_derived: false,
                }),
                NodeType::PhaseMapping => Node::PhaseMapping(id),
                NodeType::Changeset => Node::Changeset(ChangesetKey {
                    inner: id,
                    filenode_known_derived: false,
                }),
                NodeType::ChangesetInfo => Node::ChangesetInfo(id),
                NodeType::ChangesetInfoMapping => Node::ChangesetInfoMapping(id),
                NodeType::DeletedManifestMapping => Node::DeletedManifestMapping(id),
                NodeType::FsnodeMapping => Node::FsnodeMapping(id),
                NodeType::SkeletonManifestMapping => Node::SkeletonManifestMapping(id),
                NodeType::UnodeMapping => Node::UnodeMapping(id),
                _ => bail!("Unsupported root type for chunking {:?}", r),
            };
            if let Some(edge_type) = n.get_type().root_edge_type() {
                let edge = OutgoingEdge::new(edge_type, n);
                edges.push(edge);
            } else {
                bail!("Unsupported node type for root edges {:?}", n.get_type());
            }
        }
    }
    Ok(edges)
}

#[derive(Clone, Debug)]
pub struct TailParams {
    pub tail_secs: Option<u64>,
    pub public_changeset_chunk_size: Option<usize>,
    pub public_changeset_chunk_by: HashSet<NodeType>,
    pub chunk_direction: Direction,
    pub clear_interned_types: HashSet<InternedType>,
    pub clear_node_types: HashSet<NodeType>,
    pub clear_sample_rate: Option<u64>,
    pub checkpoints: Option<CheckpointsByName>,
    pub state_max_age: Duration,
    pub checkpoint_sample_rate: u64,
    pub allow_remaining_deferred: bool,
    pub repo_lower_bound_override: Option<u64>,
    pub repo_upper_bound_override: Option<u64>,
}

// Represent that only one end of the bound is optional, depending on direction
enum BestBounds {
    NewestFirst(Option<u64>, u64),
    OldestFirst(u64, Option<u64>),
}

impl BestBounds {
    // Checkpoint if necessary given the existing bounds. If there was a change return new bounds and checkpoint.
    async fn checkpoint(
        &self,
        logger: &Logger,
        repo_id: RepositoryId,
        checkpoints: &CheckpointsByName,
        chunk_low: u64,
        chunk_upper: u64,
        checkpoint: Option<Checkpoint>,
        chunk_num: u64,
    ) -> Result<Option<(BestBounds, Checkpoint)>, Error> {
        match self {
            BestBounds::NewestFirst(best_low, repo_high_bound) => {
                let new_best = best_low.map_or_else(
                    || Some(chunk_low),
                    |v| if chunk_low < v { Some(chunk_low) } else { None },
                );
                if let Some(new_best) = new_best {
                    let checkpoint = checkpoints
                        .persist(
                            logger,
                            repo_id,
                            chunk_num,
                            checkpoint,
                            new_best,
                            *repo_high_bound,
                        )
                        .await?;
                    return Ok(Some((
                        BestBounds::NewestFirst(Some(new_best), *repo_high_bound),
                        checkpoint,
                    )));
                }
            }
            BestBounds::OldestFirst(repo_low_bound, best_high) => {
                let new_best = best_high.map_or_else(
                    || Some(chunk_upper),
                    |v| {
                        if chunk_upper > v {
                            Some(chunk_upper)
                        } else {
                            None
                        }
                    },
                );
                if let Some(new_best) = new_best {
                    let checkpoint = checkpoints
                        .persist(
                            logger,
                            repo_id,
                            chunk_num,
                            checkpoint,
                            *repo_low_bound,
                            new_best,
                        )
                        .await?;
                    return Ok(Some((
                        BestBounds::OldestFirst(*repo_low_bound, Some(new_best)),
                        checkpoint,
                    )));
                }
            }
        }
        Ok(None)
    }
}

pub async fn walk_exact_tail<RunFac, SinkFac, SinkOut, V, VOut, Route>(
    fb: FacebookInit,
    job_params: JobWalkParams,
    mut repo_params: RepoWalkParams,
    type_params: RepoWalkTypeParams,
    tail_params: TailParams,
    mut visitor: V,
    make_run: RunFac,
) -> Result<(), Error>
where
    RunFac: 'static + Clone + Send + Sync + FnOnce(&CoreContext, &RepoWalkParams) -> SinkFac,
    SinkFac: 'static
        + FnOnce(BoxStream<'static, Result<VOut, Error>>, Timestamp, u64) -> SinkOut
        + Clone
        + Send,
    SinkOut: Future<Output = Result<(), Error>> + 'static + Send,
    V: 'static + TailingWalkVisitor + WalkVisitor<VOut, Route> + Send + Sync,
    VOut: 'static + Send,
    Route: 'static + Send + Clone + StepRoute,
{
    let repo_id = repo_params.repo.get_repoid();

    let mut state_start = Timestamp::now();

    let with_hg = repo_params.include_node_types.iter().any(|n| {
        let n = n.derived_data_name();
        n == Some(MappedHgChangesetId::NAME) || n == Some(FilenodesOnlyPublic::NAME)
    });

    loop {
        cloned!(job_params, tail_params, type_params, make_run);
        let tail_secs = tail_params.tail_secs;
        // Each loop get new ctx and thus session id so we can distinguish runs
        let ctx = CoreContext::new_with_logger(fb, repo_params.logger.clone());
        let session_text = ctx.session().metadata().session_id().to_string();
        if !job_params.quiet {
            info!(
                repo_params.logger,
                "Starting walk with session id {}", &session_text
            )
        }
        repo_params.scuba_builder.add("session", session_text);

        let mut checkpoint = if let Some(checkpoints) = tail_params.checkpoints.as_ref() {
            checkpoints.load(repo_id).await?
        } else {
            None
        };

        if let Some(cp) = checkpoint.as_ref() {
            info!(repo_params.logger, #log::CHUNKING, "Found checkpoint with bounds: ({}, {})", cp.lower_bound, cp.upper_bound);
        }

        let chunk_params = tail_params
            .public_changeset_chunk_size
            .map(|chunk_size| {
                let heads_fetcher = PublicChangesetBulkFetch::new(
                    repo_params.repo.get_changesets_object(),
                    repo_params.repo.get_phases(),
                )
                .with_read_from_master(false)
                .with_step(MAX_FETCH_STEP);
                heads_fetcher.map(|v| (chunk_size as usize, v))
            })
            .transpose()?;

        let is_chunking = chunk_params.is_some();
        let mut run_start = Timestamp::now();
        let mut chunk_smaller_than_fetch = false;

        // Get the chunk stream and whether the bounds it covers are contiguous
        let (contiguous_bounds, mut best_bounds, chunk_stream) = if let Some((
            chunk_size,
            heads_fetcher,
        )) = &chunk_params
        {
            if *chunk_size < MAX_FETCH_STEP as usize {
                chunk_smaller_than_fetch = true;
            }
            let (mut lower, mut upper) = heads_fetcher.get_repo_bounds(&ctx).await?;
            if let Some(lower_override) = tail_params.repo_lower_bound_override {
                lower = lower_override;
            }
            if let Some(upper_override) = tail_params.repo_upper_bound_override {
                upper = upper_override;
            }

            info!(repo_params.logger, #log::CHUNKING, "Repo bounds: ({}, {})", lower, upper);

            let (contiguous_bounds, best_bound, catchup_bounds, main_bounds) = if let Some(
                ref mut checkpoint,
            ) = checkpoint
            {
                let age_secs = checkpoint.create_timestamp.since_seconds();
                run_start = checkpoint.create_timestamp;
                if age_secs >= 0 && Duration::from_secs(age_secs as u64) > tail_params.state_max_age
                {
                    info!(repo_params.logger, #log::CHUNKING, "Checkpoint run {} chunk {} is too old at {}s, running from repo bounds",
                        checkpoint.update_run_number, checkpoint.update_chunk_number, age_secs);
                    // Increment checkpoints run, reset chunk and create_timestamp as we're starting again
                    checkpoint.update_run_number += 1;
                    checkpoint.update_chunk_number = 0;
                    checkpoint.create_timestamp = Timestamp::now();
                    run_start = checkpoint.create_timestamp;
                    (true, None, None, Some((lower, upper)))
                } else {
                    let (catchup_bounds, main_bounds) =
                        checkpoint.stream_bounds(lower, upper, tail_params.chunk_direction)?;

                    let contiguous_bounds =
                        match (tail_params.chunk_direction, catchup_bounds, main_bounds) {
                            (
                                Direction::NewestFirst,
                                Some((catchup_lower, _)),
                                Some((_, main_upper)),
                            ) => catchup_lower == main_upper,
                            (
                                Direction::OldestFirst,
                                Some((_, catchup_upper)),
                                Some((main_lower, _)),
                            ) => catchup_upper == main_lower,
                            (_, Some(_), None) => false,
                            _ => true,
                        };
                    info!(repo_params.logger, #log::CHUNKING, "Continuing from checkpoint run {} chunk {} with catchup {:?} and main {:?} bounds",
                        checkpoint.update_run_number, checkpoint.update_chunk_number, catchup_bounds, main_bounds);
                    (
                        contiguous_bounds,
                        if tail_params.chunk_direction == Direction::NewestFirst {
                            Some(checkpoint.lower_bound)
                        } else {
                            Some(checkpoint.upper_bound)
                        },
                        catchup_bounds,
                        main_bounds,
                    )
                }
            } else {
                (true, None, None, Some((lower, upper)))
            };

            let load_ids = |(lower, upper)| {
                heads_fetcher
                    .fetch_ids(&ctx, tail_params.chunk_direction, Some((lower, upper)))
                    .chunks(*chunk_size)
                    .map(move |v| v.into_iter().collect::<Result<HashSet<_>, Error>>())
            };

            let main_s = if let Some(main_bounds) = main_bounds {
                load_ids(main_bounds).left_stream()
            } else {
                stream::once(future::ok(HashSet::new())).right_stream()
            };

            let s = if let Some(catchup_bounds) = catchup_bounds {
                load_ids(catchup_bounds).chain(main_s).left_stream()
            } else {
                main_s.right_stream()
            };

            let best_bounds = if tail_params.chunk_direction == Direction::NewestFirst {
                BestBounds::NewestFirst(best_bound, upper)
            } else {
                BestBounds::OldestFirst(lower, best_bound)
            };

            (contiguous_bounds, Some(best_bounds), s.left_stream())
        } else {
            let s = stream::once(future::ok(HashSet::new())).right_stream();
            (true, None, s)
        };

        let mut chunk_num: u64 = 0;
        if let Some(checkpoint) = checkpoint.as_ref() {
            chunk_num = checkpoint.update_chunk_number;
        }

        let mut last_chunk_low = None;
        let mut last_chunk_upper = None;

        futures::pin_mut!(chunk_stream);
        while let Some(chunk_members) = chunk_stream.try_next().await? {
            if is_chunking && chunk_members.is_empty() {
                continue;
            }
            chunk_num += 1;

            // convert from stream of (id, bounds) to ids plus overall bounds
            let mut chunk_low: u64 = u64::MAX;
            let mut chunk_upper: u64 = 0;
            let chunk_members: HashSet<ChangesetId> = chunk_members
                .into_iter()
                .map(|((cs_id, id), (fetch_low, fetch_upper))| {
                    if chunk_smaller_than_fetch {
                        // Adjust the bounds so it doesn't exceed previous chunk
                        if tail_params.chunk_direction == Direction::NewestFirst {
                            chunk_low = min(chunk_low, id);
                            chunk_upper = max(chunk_upper, fetch_upper);
                            if let Some(last_chunk_low) = last_chunk_low {
                                chunk_upper = min(last_chunk_low, chunk_upper)
                            }
                        } else {
                            chunk_low = min(chunk_low, fetch_low);
                            if let Some(last_chunk_upper) = last_chunk_upper {
                                chunk_low = max(last_chunk_upper, chunk_low)
                            }
                            chunk_upper = max(chunk_upper, fetch_upper);
                        }
                    } else {
                        // no need to adjust
                        chunk_low = min(chunk_low, fetch_low);
                        chunk_upper = max(chunk_upper, fetch_upper);
                    }
                    cs_id
                })
                .collect();

            last_chunk_low.replace(chunk_low);
            last_chunk_upper.replace(chunk_upper);
            cloned!(repo_params.logger);
            if is_chunking {
                info!(logger, #log::CHUNKING, "Starting chunk {} with bounds ({}, {})", chunk_num, chunk_low, chunk_upper);
            }

            cloned!(mut repo_params);
            let hg_mapping_prepop = if with_hg && is_chunking {
                // bulk prepopulate the hg/bonsai mappings
                let ids =
                    BonsaiOrHgChangesetIds::Bonsai(chunk_members.clone().into_iter().collect());
                repo_params
                    .repo
                    .bonsai_hg_mapping()
                    .get(&ctx, repo_id, ids)
                    .await?
            } else {
                vec![]
            };

            let extra_roots = visitor
                .start_chunk(&chunk_members, hg_mapping_prepop)?
                .into_iter();
            let chunk_roots =
                roots_for_chunk(chunk_members, &tail_params.public_changeset_chunk_by)?;
            repo_params.walk_roots.extend(chunk_roots);
            repo_params.walk_roots.extend(extra_roots);

            cloned!(ctx, job_params, make_run, type_params);
            let make_sink = make_run(&ctx, &repo_params);

            // Walk needs clonable visitor, so wrap in Arc for its duration
            let arc_v = Arc::new(visitor);
            let walk_output =
                walk_exact(ctx, arc_v.clone(), job_params, repo_params, type_params).boxed();
            make_sink(walk_output, run_start, chunk_num).await?;
            visitor = Arc::try_unwrap(arc_v).map_err(|_| anyhow!("could not unwrap visitor"))?;

            if is_chunking {
                info!(logger, #log::LOADED, "Deferred: {}", visitor.num_deferred());
                if let Some(sample_rate) = tail_params.clear_sample_rate {
                    if sample_rate != 0 && chunk_num % sample_rate == 0 {
                        info!(logger, #log::CHUNKING, "Clearing state after chunk {}", chunk_num);
                        visitor.clear_state(
                            &tail_params.clear_node_types,
                            &tail_params.clear_interned_types,
                        );
                    }
                }

                // Record checkpoint and update best_bounds
                if let Some(checkpoints) = tail_params.checkpoints.as_ref() {
                    if tail_params.checkpoint_sample_rate != 0
                        && chunk_num % tail_params.checkpoint_sample_rate == 0
                    {
                        let maybe_new = if let Some(best_bounds) = best_bounds.as_ref() {
                            best_bounds
                                .checkpoint(
                                    &logger,
                                    repo_id,
                                    checkpoints,
                                    chunk_low,
                                    chunk_upper,
                                    checkpoint.clone(),
                                    chunk_num,
                                )
                                .await?
                        } else {
                            None
                        };
                        if let Some((new_best, new_cp)) = maybe_new {
                            checkpoint.replace(new_cp);
                            best_bounds.replace(new_best);
                        }
                    }
                }
            }
        }

        visitor.end_chunks(
            &repo_params.logger,
            contiguous_bounds
                && !tail_params.allow_remaining_deferred
                // If lower bound overridden then not contiguous to repo start. Overriding upper bound should not result in deferred.
                && ((tail_params.chunk_direction == Direction::NewestFirst && tail_params.repo_lower_bound_override.is_none())
                    || (tail_params.chunk_direction == Direction::OldestFirst && tail_params.repo_upper_bound_override.is_none())),
        )?;

        match (tail_params.checkpoints.as_ref(), checkpoint.as_ref()) {
            (Some(checkpoints), Some(cp)) => checkpoints.finish(repo_id, cp).await?,
            _ => {}
        }

        if let Some((chunk_size, _heads_fetcher)) = &chunk_params {
            info!(
                repo_params.logger, #log::CHUNKING,
                "Completed in {} chunks of size {}", chunk_num, chunk_size
            );
        };

        match tail_secs {
            Some(interval) => {
                let start = Instant::now();
                let next_iter_deadline = start + Duration::from_secs(interval);
                tokio::time::sleep_until(next_iter_deadline).await;
                let age_secs = state_start.since_seconds();
                if age_secs >= 0 && Duration::from_secs(age_secs as u64) > tail_params.state_max_age
                {
                    // Walk state is too old, clear it.
                    info!(
                        repo_params.logger,
                        "Clearing walk state after {} seconds", age_secs
                    );
                    visitor
                        .clear_state(&NodeType::iter().collect(), &InternedType::iter().collect());
                    state_start = Timestamp::now();
                }
            }
            None => return Ok(()),
        }
    }
}
