// SPDX-License-Identifier: Apache-2.0
//! Pool bearer admission. Server retains caller and tenant authorization.

use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;

use axum::http::{HeaderMap, header};
use subtle::ConstantTimeEq;

const BEARER_LEN: usize = 64;

#[derive(Clone)]
pub(crate) struct Bearer(Arc<[u8; BEARER_LEN]>);

impl Bearer {
    pub(crate) fn read(path: &Path) -> io::Result<Self> {
        let error = || {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "LENS_SERVICE_BEARER_FILE must be a readable file containing 64 lowercase hexadecimal characters",
            )
        };
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| error())?
            .take((BEARER_LEN + 3) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| error())?;
        let secret = bytes
            .strip_suffix(b"\r\n")
            .or_else(|| bytes.strip_suffix(b"\n"))
            .unwrap_or(&bytes);
        let secret: [u8; BEARER_LEN] = secret.try_into().map_err(|_| error())?;
        if !secret
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        {
            return Err(error());
        }
        Ok(Self(Arc::new(secret)))
    }

    pub(crate) fn admits(&self, headers: &HeaderMap) -> bool {
        let mut headers = headers.get_all(header::AUTHORIZATION).iter();
        let Some(value) = headers.next().and_then(|value| value.to_str().ok()) else {
            return false;
        };
        if headers.next().is_some() {
            return false;
        }
        let Some((scheme, token)) = value.split_once(' ') else {
            return false;
        };
        scheme.eq_ignore_ascii_case("bearer")
            && bool::from(
                self.0
                    .as_slice()
                    .ct_eq(token.trim_start_matches(' ').as_bytes()),
            )
    }
}
