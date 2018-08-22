extern crate rayon;

use chan;
use filetime::FileTime;
use miner::Buffer;
use plot::Plot;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Sender, TryRecvError};
use std::sync::{Arc, Mutex};

pub struct ReadReply {
    pub buffer: Box<Buffer + Send>,
    pub len: usize,
    pub height: u64,
    pub gensig: Arc<[u8; 32]>,
    pub start_nonce: u64,
    pub finished: bool,
}

pub struct Reader {
    drive_id_to_plots: HashMap<String, Arc<Mutex<Vec<RefCell<Plot>>>>>,
    pool: rayon::ThreadPool,
    rx_empty_buffers: chan::Receiver<Box<Buffer + Send>>,
    tx_read_replies_cpu: chan::Sender<ReadReply>,
    tx_read_replies_gpu: chan::Sender<ReadReply>,
    interupts: Vec<Sender<()>>,
}

impl Reader {
    pub fn new(
        drive_id_to_plots: HashMap<String, Arc<Mutex<Vec<RefCell<Plot>>>>>,
        num_threads: usize,
        rx_empty_buffers: chan::Receiver<Box<Buffer + Send>>,
        tx_read_replies_cpu: chan::Sender<ReadReply>,
        tx_read_replies_gpu: chan::Sender<ReadReply>,
    ) -> Reader {
        for plots in drive_id_to_plots.values() {
            let mut plots = plots.lock().unwrap();
            plots.sort_by_key(|p| {
                let m = p.borrow().fh.metadata().unwrap();
                -FileTime::from_last_modification_time(&m).unix_seconds()
            });
        }

        Reader {
            drive_id_to_plots,
            pool: rayon::ThreadPoolBuilder::new()
                .num_threads(num_threads)
                .build()
                .unwrap(),
            rx_empty_buffers,
            tx_read_replies_cpu,
            tx_read_replies_gpu,
            interupts: Vec::new(),
        }
    }

    pub fn start_reading(&mut self, height: u64, scoop: u32, gensig: &Arc<[u8; 32]>) {
        for interupt in &self.interupts {
            interupt.send(()).ok();
        }
        self.interupts = self
            .drive_id_to_plots
            .iter()
            .map(|(_, plots)| {
                let (interupt, task) =
                    self.create_read_task(plots.clone(), height, scoop, gensig.clone());
                self.pool.spawn(task);
                interupt
            }).collect();
    }

    pub fn wakeup(&mut self) {
        for plots in self.drive_id_to_plots.values() {
            let plots = plots.clone();
            self.pool.spawn(move || {
                let plots = plots.lock().unwrap();
                let mut p = plots[0].borrow_mut();

                if let Err(e) = p.seek_random() {
                    error!(
                        "wakeup: error during wakeup {}: {} -> skip one round",
                        p.name, e
                    );
                }
            });
        }
    }

    fn create_read_task(
        &self,
        plots: Arc<Mutex<Vec<RefCell<Plot>>>>,
        height: u64,
        scoop: u32,
        gensig: Arc<[u8; 32]>,
    ) -> (Sender<()>, impl FnOnce()) {
        let (tx_interupt, rx_interupt) = channel();
        let rx_empty_buffers = self.rx_empty_buffers.clone();
        let tx_read_replies_cpu = self.tx_read_replies_cpu.clone();
        let tx_read_replies_gpu = self.tx_read_replies_gpu.clone();

        (tx_interupt, move || {
            let plots = plots.lock().unwrap();
            let plot_count = plots.len();
            'outer: for (i_p, p) in plots.iter().enumerate() {
                let mut p = p.borrow_mut();
                if let Err(e) = p.prepare(scoop) {
                    error!(
                        "reader: error preparing {} for reading: {} -> skip one round",
                        p.name, e
                    );
                    continue 'outer;
                }

                'inner: for mut buffer in rx_empty_buffers.clone() {
                    let mut_bs = &*buffer.get_buffer_for_writing();
                    let mut bs = mut_bs.lock().unwrap();
                    let (bytes_read, start_nonce, next_plot) = match p.read(&mut *bs, scoop) {
                        Ok(x) => x,
                        Err(e) => {
                            error!(
                                "reader: error reading chunk from {}: {} -> skip one round",
                                p.name, e
                            );
                            (0, 0, true)
                        }
                    };

                    let finished = i_p == (plot_count - 1) && next_plot;

                    //fork cpu / gpu
                    if buffer.is_gpu() {
                        tx_read_replies_gpu.send(ReadReply {
                            buffer: buffer,
                            len: bytes_read,
                            height,
                            gensig: gensig.clone(),
                            start_nonce,
                            finished,
                        });
                    } else {
                        tx_read_replies_cpu.send(ReadReply {
                            buffer: buffer,
                            len: bytes_read,
                            height,
                            gensig: gensig.clone(),
                            start_nonce,
                            finished,
                        });
                    }

                    if next_plot {
                        break 'inner;
                    }
                    if rx_interupt.try_recv() != Err(TryRecvError::Empty) {
                        break 'outer;
                    }
                }
            }
        })
    }
}
