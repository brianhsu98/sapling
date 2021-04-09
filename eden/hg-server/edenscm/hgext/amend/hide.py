# Portions Copyright (c) Facebook, Inc. and its affiliates.
#
# This software may be used and distributed according to the terms of the
# GNU General Public License version 2.

# hide.py - simple and user-friendly commands to hide and unhide commits
#
# Copyright 2011 Peter Arrenbrecht <peter.arrenbrecht@gmail.com>
#                Logilab SA        <contact@logilab.fr>
#                Pierre-Yves David <pierre-yves.david@ens-lyon.org>
#                Patrick Mezard <patrick@mezard.eu>
#
# This software may be used and distributed according to the terms of the
# GNU General Public License version 2 or any later version.
from __future__ import absolute_import

from edenscm.mercurial import (
    bookmarks as bookmarksmod,
    cmdutil,
    error,
    extensions,
    hg,
    hintutil,
    node as nodemod,
    obsolete,
    registrar,
    scmutil,
    visibility,
)
from edenscm.mercurial.i18n import _, _n
from edenscm.mercurial.node import short


cmdtable = {}
command = registrar.command(cmdtable)


@command(
    "hide|strip|hid|str|stri",
    [
        ("r", "rev", [], _("revisions to hide")),
        (
            "c",
            "cleanup",
            None,
            _(
                "clean up commits with newer versions, and non-essential remote bookmarks"
            ),
        ),
        ("B", "bookmark", [], _("hide commits only reachable from a bookmark")),
    ],
    _("[OPTION]... [-r] REV..."),
)
def hide(ui, repo, *revs, **opts):
    """hide commits and their descendants

    Mark the specified commits as hidden. Hidden commits are not included in
    the output of most Mercurial commands, including :hg:`log` and
    :hg:`smartlog.` Any descendants of the specified commits will also be
    hidden.

    Hidden commits are not deleted. They will remain in the repo indefinitely
    and are still accessible by their hashes. However, :hg:`hide` will delete
    any bookmarks pointing to hidden commits.

    Use the :hg:`unhide` command to make hidden commits visible again. See
    :hg:`help unhide` for more information.

    To view hidden commits, run :hg:`journal`.

    When you hide the current commit, the most recent visible ancestor is
    checked out.

    To hide obsolete stacks (stacks that have a newer version), run
    :hg:`hide --cleanup`. This command is equivalent to:

    :hg:`hide 'obsolete() - ancestors(draft() & not obsolete())'`

    --cleanup skips obsolete commits with non-obsolete descendants.
    """
    removednames = []
    if opts.get("cleanup") and len(opts.get("rev") + list(revs)) != 0:
        raise error.Abort(_("--rev and --cleanup are incompatible"))
    elif opts.get("cleanup"):
        # cleanup remote bookmarks
        removednames = bookmarksmod.cleanupremotenames(repo)
        # hides all the draft, obsolete commits that
        # don't have non-obsolete descendants
        revs = ["obsolete() - (draft() & ::(draft() & not obsolete()))"]
    else:
        revs = list(revs) + opts.pop("rev", [])

    with repo.wlock(), repo.lock(), repo.transaction("hide") as tr:
        revs = repo.revs("(%ld)::", scmutil.revrange(repo, revs))

        bookmarks = set(opts.get("bookmark", ()))
        remotebooks = dict()

        # allow remote scratch bookmarks
        if repo.ui.configbool("remotenames", "selectivepull"):
            rb = repo._remotenames.mark2nodes()
            for bookmark in bookmarks:
                if bookmark in rb:
                    node = rb[bookmark][0]
                    if repo[node].mutable():
                        remotebooks[bookmark] = node
            draftnodes = repo.dageval(lambda: ancestors(remotebooks.values()) & draft())
            revs += repo.changelog.torevset(draftnodes)

        bookmarks = bookmarks - set(remotebooks.keys())
        if bookmarks:
            revs += bookmarksmod.reachablerevs(repo, bookmarks)
            if not revs:
                # No revs are reachable exclusively from these bookmarks, just
                # delete the bookmarks.
                if not ui.quiet:
                    for bookmark in sorted(bookmarks):
                        ui.status(
                            _("removing bookmark '%s' (was at: %s)\n")
                            % (bookmark, short(repo._bookmarks[bookmark]))
                        )
                bookmarksmod.delete(repo, tr, bookmarks)
                ui.status(
                    _n(
                        "%i bookmark removed\n",
                        "%i bookmarks removed\n",
                        len(bookmarks),
                    )
                    % len(bookmarks)
                )
                return 0

        if not revs:
            if removednames:
                return 0
            raise error.Abort(_("nothing to hide"))

        hidectxs = [repo[r] for r in revs]

        # revs to be hidden
        for ctx in hidectxs:
            if not ctx.mutable():
                raise error.Abort(
                    _("cannot hide immutable changeset: %s") % ctx,
                    hint="see 'hg help phases' for details",
                )
            if not ui.quiet:
                ui.status(
                    _('hiding commit %s "%s"\n')
                    % (ctx, ctx.description().split("\n")[0][:50])
                )

        wdp = repo["."]
        newnode = wdp

        while newnode in hidectxs:
            newnode = newnode.parents()[0]

        if newnode.node() != wdp.node():
            cmdutil.bailifchanged(repo, merge=False)
            hg.update(repo, newnode, False)
            ui.status(
                _("working directory now at %s\n") % ui.label(str(newnode), "node")
            )

        # create markers
        if obsolete.isenabled(repo, obsolete.createmarkersopt):
            obsolete.createmarkers(repo, [(r, []) for r in hidectxs], operation="hide")
        visibility.remove(repo, [c.node() for c in hidectxs])
        ui.status(
            _n("%i changeset hidden\n", "%i changesets hidden\n", len(hidectxs))
            % len(hidectxs)
        )

        # remove bookmarks pointing to hidden changesets
        hnodes = [r.node() for r in hidectxs]
        deletebookmarks = set(bookmarks)
        for bookmark, node in sorted(bookmarksmod.listbinbookmarks(repo)):
            if node in hnodes:
                deletebookmarks.add(bookmark)
        if deletebookmarks:
            for bookmark in sorted(deletebookmarks):
                if not ui.quiet:
                    ui.status(
                        _('removing bookmark "%s (was at: %s)"\n')
                        % (bookmark, short(repo._bookmarks[bookmark]))
                    )
            bookmarksmod.delete(repo, tr, deletebookmarks)
            ui.status(
                _n(
                    "%i bookmark removed\n",
                    "%i bookmarks removed\n",
                    len(deletebookmarks),
                )
                % len(deletebookmarks)
            )
        # unsubscribe from the remote bookmarks pointing to hidden changesets
        # they always will be remote scratch bookmarks because hidectxs are all draft
        if ui.configbool("remotenames", "selectivepull"):
            node2marks = repo._remotenames.node2marks()
            marks = sum([node2marks[node] for node in hnodes if node in node2marks], [])
            bmremove = {key: nodemod.nullhex for key in marks}
            if bmremove:
                for bookmark in sorted(marks):
                    if not ui.quiet:
                        ui.status(_('unsubscribing remote bookmark "%s"\n') % bookmark)
                repo._remotenames.applychanges({"bookmarks": bmremove}, override=False)
                ui.status(
                    _n(
                        "%i remote bookmark unsubscribed\n",
                        "%i remote bookmarks unsubscribed\n",
                        len(bmremove),
                    )
                    % len(bmremove)
                )
        hintutil.trigger("undo")


@command(
    "unhide|unh|unhi|unhid",
    [("r", "rev", [], _("revisions to unhide"))],
    _("[OPTION]... [-r] REV..."),
)
def unhide(ui, repo, *revs, **opts):
    """unhide commits and their ancestors

    Mark the specified commits as visible. Any ancestors of the specified
    commits will also become visible.
    """
    revs = list(revs) + opts.pop("rev", [])
    with repo.wlock(), repo.lock():
        revs = set(scmutil.revrange(repo, revs))
        _dounhide(repo, revs)


def _dounhide(repo, revs):
    unfi = repo
    if obsolete.isenabled(repo, obsolete.createmarkersopt):
        ctxs = unfi.set("not public() & ::(%ld) & obsolete()", revs)
        obsolete.revive(ctxs, operation="unhide")
    visibility.add(repo, [unfi[r].node() for r in revs])
