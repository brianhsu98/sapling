/*
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * This software may be used and distributed according to the terms of the
 * GNU General Public License version 2.
 */

use crate::HgId;
use serde::Serializer;

/// Serde `serialize_with` function to serialize `HgId` as hex string.
pub fn serialize<S>(id: &HgId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let hex = id.to_hex();
    serializer.serialize_str(&hex)
}

// bytes::deserialize can decode hex.
pub use super::bytes::deserialize;
