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

/// `read` of every item, all of them at once, in the items' order.
///
/// For the subsets of one window: a window across 180° is two reads of the
/// same step, and each is a request with its own round trip. Every read
/// completes before this returns, as with [`try_join`], and the first error
/// in order refuses the lot. Bounded by the window: never more than two.
pub(crate) fn try_all<I: Sync, O: Send>(
    items: &[I],
    read: impl Fn(&I) -> Result<O> + Sync,
) -> Result<Vec<O>> {
    let Some((last, rest)) = items.split_last() else {
        return Ok(Vec::new());
    };
    let read = &read;
    std::thread::scope(|scope| {
        let spawned: Vec<_> = rest
            .iter()
            .map(|item| scope.spawn(move || read(item)))
            .collect();
        let last = read(last);
        let mut results: Vec<Result<O>> = spawned
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect();
        results.push(last);
        results.into_iter().collect()
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
    fn the_subsets_of_a_window_are_read_together() {
        // Each read hands the other a token and waits for the other's: read
        // one after the other, the first waits for ever.
        let (to_second, from_first) = channel();
        let (to_first, from_second) = channel();
        let ends = [
            (
                std::sync::Mutex::new(to_second),
                std::sync::Mutex::new(from_second),
            ),
            (
                std::sync::Mutex::new(to_first),
                std::sync::Mutex::new(from_first),
            ),
        ];
        let read = |k: &usize| {
            let (tx, rx) = &ends[*k];
            tx.lock()
                .expect("lock")
                .send(*k)
                .expect("the other is listening");
            rx.lock()
                .expect("lock")
                .recv_timeout(Duration::from_secs(5))
                .map_err(|e| ZarrError::Open(e.to_string()))?;
            Ok(*k * 10)
        };
        assert_eq!(try_all(&[0usize, 1], read).expect("both complete"), [0, 10]);
        let failing = try_all(&[0usize, 1, 2], |k| {
            if *k == 0 {
                Ok(0)
            } else {
                Err(ZarrError::Open(format!("subset {k}")))
            }
        });
        assert!(
            failing
                .expect_err("refused")
                .to_string()
                .contains("subset 1")
        );
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
