use std::num::TryFromIntError;
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum NowTsError {
    #[error("system time before unix epoch")]
    BeforeUnixEpoch(#[source] SystemTimeError),
    #[error("system time does not fit in i64")]
    OutOfRange(#[source] TryFromIntError),
}

pub fn now_ts() -> Result<i64, NowTsError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(NowTsError::BeforeUnixEpoch)?;
    let secs = i64::try_from(duration.as_secs()).map_err(NowTsError::OutOfRange)?;
    Ok(secs)
}
