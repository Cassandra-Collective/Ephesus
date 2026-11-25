//! One outstanding write, independent of the terminal's bounded output drain.

use std::{
    io::{self, Write},
    sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError},
    thread,
};

pub(crate) type Writer = Box<dyn Write + Send>;

pub(crate) struct Input {
    requests: SyncSender<Vec<u8>>,
    results: Receiver<io::Result<()>>,
    pending: bool,
}

impl Input {
    pub(crate) fn start() -> io::Result<(Sender<Writer>, Self)> {
        let (attach, writer) = mpsc::channel::<Writer>();
        let (requests, queued) = mpsc::sync_channel::<Vec<u8>>(1);
        let (reported, results) = mpsc::sync_channel(1);
        let _worker = thread::Builder::new()
            .name("eph-pty-input".into())
            .spawn(move || {
                let Ok(mut writer) = writer.recv() else {
                    return;
                };
                for bytes in queued {
                    if reported.send(writer.write_all(&bytes)).is_err() {
                        break;
                    }
                }
            })?;
        Ok((
            attach,
            Self {
                requests,
                results,
                pending: false,
            },
        ))
    }

    pub(crate) fn queue(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        if self.pending {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "the terminal has an outstanding input write",
            ));
        }
        self.requests.try_send(bytes).map_err(|_| {
            io::Error::new(io::ErrorKind::BrokenPipe, "the terminal input worker ended")
        })?;
        self.pending = true;
        Ok(())
    }

    pub(crate) fn result(&mut self) -> Option<io::Result<()>> {
        if !self.pending {
            return None;
        }
        let result = match self.results.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the terminal input worker ended",
            )),
        };
        self.pending = false;
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct BlockedWriter {
        started: Sender<Vec<u8>>,
        ready: Receiver<()>,
    }

    #[test]
    fn a_lost_worker_refuses_a_write_and_reports_an_unacknowledged_one_once() {
        let (requests, queued) = mpsc::sync_channel(1);
        let (reported, results) = mpsc::sync_channel(1);
        let mut input = Input {
            requests,
            results,
            pending: false,
        };
        input.queue(b"not acknowledged".to_vec()).unwrap();
        assert_eq!(queued.try_recv().unwrap(), b"not acknowledged");
        drop(reported);
        assert_eq!(
            input.result().unwrap().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert!(input.result().is_none());
        drop(queued);
        assert_eq!(
            input.queue(b"no worker".to_vec()).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    impl Write for BlockedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.started.send(bytes.to_vec()).unwrap();
            self.ready.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_blocked_writer_acknowledges_only_after_drain_and_rejects_another_write() {
        let (attach, mut input) = Input::start().unwrap();
        let (started, bytes) = mpsc::channel();
        let (unblock, ready) = mpsc::channel();
        assert!(
            attach
                .send(Box::new(BlockedWriter { started, ready }))
                .is_ok()
        );
        assert!(input.result().is_none());
        input.queue(b"first".to_vec()).unwrap();
        assert_eq!(
            bytes.recv_timeout(Duration::from_secs(5)).unwrap(),
            b"first"
        );
        assert!(input.result().is_none());
        assert_eq!(
            input.queue(b"too soon".to_vec()).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        unblock.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = input.result() {
                result.unwrap();
                break;
            }
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        assert!(input.result().is_none());
    }
}
