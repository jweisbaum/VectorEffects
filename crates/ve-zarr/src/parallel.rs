//! Small, bounded groups of blocking archive reads. These run on OS threads,
//! never the renderer's Rayon pool or a Tokio runtime worker.

use crate::Result;

/// Complete both reads before returning, including when either fails. Callers
/// nest this only for fixed groups: at most eight HTTP requests per import.
pub(crate) fn try_join<A: Send, B>(
    left: impl FnOnce() -> Result<A> + Send,
    right: impl FnOnce() -> Result<B>,
) -> Result<(A, B)> {
    std::thread::scope(|scope| {
        let left = scope.spawn(left);
        let right = right();
        let left = left
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        Ok((left?, right?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ZarrError;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[test]
    fn independent_reads_can_make_progress_together() {
        let (tx, rx) = channel();
        let pair = try_join(
            move || {
                rx.recv_timeout(Duration::from_secs(5))
                    .map_err(|e| ZarrError::Open(e.to_string()))?;
                Ok("eastward")
            },
            || {
                tx.send(()).expect("reader is waiting");
                Ok("northward")
            },
        )
        .expect("both complete");
        assert_eq!(pair, ("eastward", "northward"));
    }

    #[test]
    fn failure_in_either_component_refuses_the_pair() {
        for fails_left in [false, true] {
            let read = |fails| {
                if fails {
                    Err(ZarrError::Open("missing component".into()))
                } else {
                    Ok(7)
                }
            };
            let result = try_join(|| read(fails_left), || read(!fails_left));
            assert!(
                result
                    .expect_err("incomplete vector must fail")
                    .to_string()
                    .contains("missing component")
            );
        }
    }
}
