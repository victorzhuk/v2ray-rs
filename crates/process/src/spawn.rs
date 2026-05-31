use std::time::Duration;

use tokio::process::{Child, Command};
use tokio::time::sleep;

/// Spawns a child process, retrying on `ETXTBSY` which can occur on overlayfs
/// (e.g. Docker containers) when a binary is written and immediately executed.
///
/// `build` is invoked once per attempt so a fresh `Command` (including any
/// `pre_exec` hooks) is constructed each time.
pub(crate) async fn spawn_with_etxtbsy_retry<F>(mut build: F) -> std::io::Result<Child>
where
    F: FnMut() -> Command,
{
    const MAX_RETRIES: u32 = 5;
    for attempt in 0..MAX_RETRIES {
        match build().spawn() {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                if attempt == MAX_RETRIES - 1 {
                    return Err(e);
                }
                sleep(Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}
