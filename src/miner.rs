extern crate aligned_alloc;
extern crate num_cpus;
extern crate ocl_core as core;
extern crate page_size;

use burstmath;
use chan;
use config::Cfg;
use core_affinity;
use futures::sync::mpsc;
use ocl::GpuBuffer;
use ocl::GpuContext;
use plot::{Plot, SCOOP_SIZE};
use reader::Reader;
use requests::RequestHandler;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::read_dir;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use std::u64;
use stopwatch::Stopwatch;
use tokio::prelude::*;
use tokio::timer::Interval;
use tokio_core::reactor::Core;
use utils::get_device_id;
use worker::{create_worker_task, NonceData};

pub struct Miner {
    reader: Reader,
    request_handler: RequestHandler,
    rx_nonce_data: mpsc::Receiver<NonceData>,
    account_id: u64,
    target_deadline: u64,
    state: Arc<Mutex<State>>,
    reader_task_count: usize,
    get_mining_info_interval: u64,
    core: Core,
    wakeup_after: i64,
}

pub struct State {
    height: u64,
    best_deadline: u64,
    base_target: u64,
    sw: Stopwatch,
    scanning: bool,

    // count how many reader's scoops have been processed
    processed_reader_tasks: usize,
}

pub trait Buffer {
    // Static method signature; `Self` refers to the implementor type.
    fn new(buffer_size: usize) -> Self
    where
        Self: Sized;
    // Instance method signatures; these will return a string.
    fn get_buffer_for_reading(&mut self) -> Arc<Mutex<Vec<u8>>>;
    fn get_buffer_for_writing(&mut self) -> Arc<Mutex<Vec<u8>>>;
    fn get_gpu_buffers(&self) -> Option<&GpuBuffer>;
    fn is_gpu(&self) -> bool;
}

pub struct CpuBuffer {
    data: Arc<Mutex<Vec<u8>>>,
}

impl Buffer for CpuBuffer {
    fn new(buffer_size: usize) -> Self
    where
        Self: Sized,
    {
        let pointer = aligned_alloc::aligned_alloc(buffer_size, page_size::get());
        let data: Vec<u8>;
        unsafe {
            data = Vec::from_raw_parts(pointer as *mut u8, buffer_size, buffer_size);
        }
        CpuBuffer {
            data: Arc::new(Mutex::new(data)),
        }
    }
    fn get_buffer_for_reading(&mut self) -> Arc<Mutex<Vec<u8>>> {
        self.data.clone()
    }
    fn get_buffer_for_writing(&mut self) -> Arc<Mutex<Vec<u8>>> {
        self.data.clone()
    }
    fn get_gpu_buffers(&self) -> Option<&GpuBuffer> {
        None
    }
    fn is_gpu(&self) -> bool {
        false
    }
}

fn scan_plots(
    plot_dirs: &[String],
    use_direct_io: bool,
) -> HashMap<String, Arc<Mutex<Vec<RefCell<Plot>>>>> {
    let mut drive_id_to_plots: HashMap<String, Arc<Mutex<Vec<RefCell<Plot>>>>> = HashMap::new();
    let mut global_capacity: f64 = 0.0;

    for plot_dir_str in plot_dirs {
        let dir = Path::new(plot_dir_str);

        if !dir.exists() {
            warn!("path {} does not exist", plot_dir_str);
            continue;
        }
        if !dir.is_dir() {
            warn!("path {} is not a directory", plot_dir_str);
            continue;
        }

        let mut num_plots = 0;
        let mut local_capacity: f64 = 0.0;
        for file in read_dir(dir).unwrap() {
            let file = &file.unwrap().path();

            if let Ok(p) = Plot::new(file, use_direct_io) {
                let drive_id = get_device_id(&file.to_str().unwrap().to_string());
                let plots = drive_id_to_plots
                    .entry(drive_id)
                    .or_insert_with(|| Arc::new(Mutex::new(Vec::new())));

                local_capacity += p.nonces as f64;
                plots.lock().unwrap().push(RefCell::new(p));
                num_plots += 1;
            }
        }

        info!(
            "path={}, files={}, size={:.4} TiB",
            plot_dir_str,
            num_plots,
            local_capacity / 4.0 / 1024.0 / 1024.0
        );

        global_capacity += local_capacity;
        if num_plots == 0 {
            warn!("no plots in {}", plot_dir_str);
        }
    }

    info!(
        "plot files loaded: total capacity={:.4} TiB",
        global_capacity / 4.0 / 1024.0 / 1024.0
    );

    drive_id_to_plots
}

impl Miner {
    pub fn new(cfg: Cfg) -> Miner {
        let drive_id_to_plots = scan_plots(&cfg.plot_dirs, cfg.hdd_use_direct_io);

        let reader_thread_count = if cfg.hdd_reader_thread_count == 0 {
            drive_id_to_plots.len()
        } else {
            cfg.hdd_reader_thread_count
        };

        let cpu_worker_thread_count = cfg.cpu_worker_thread_count;
        let gpu_worker_thread_count = cfg.gpu_worker_thread_count;

        let buffer_count = cpu_worker_thread_count * 2 + gpu_worker_thread_count * 2;
        let buffer_size_cpu = cfg.cpu_nonces_per_cache * SCOOP_SIZE as usize;

        let dummycontext;
        let mut buffer_size_gpu = 0;
        if gpu_worker_thread_count > 0 {
            dummycontext =
                GpuContext::new(cfg.gpu_platform, cfg.gpu_device, cfg.gpu_nonces_per_cache);
            buffer_size_gpu = dummycontext.gdim1[0] * SCOOP_SIZE as usize;
        }

        let (tx_empty_buffers, rx_empty_buffers) = chan::bounded(buffer_count as usize);
        let (tx_read_replies_cpu, rx_read_replies_cpu) = chan::bounded(cpu_worker_thread_count * 2);
        let (tx_read_replies_gpu, rx_read_replies_gpu) = chan::bounded(gpu_worker_thread_count * 2);

        for _ in 0..gpu_worker_thread_count * 2 {
            let gpu_buffer = GpuBuffer::new(buffer_size_gpu);
            tx_empty_buffers.send(Box::new(gpu_buffer) as Box<Buffer + Send>);
        }

        for _ in 0..cpu_worker_thread_count * 2 {
            let cpu_buffer = CpuBuffer::new(buffer_size_cpu);
            tx_empty_buffers.send(Box::new(cpu_buffer) as Box<Buffer + Send>);
        }

        let core_ids = core_affinity::get_core_ids().unwrap();
        let (tx_nonce_data, rx_nonce_data) =
            mpsc::channel(cpu_worker_thread_count + gpu_worker_thread_count);

        for id in 0..cpu_worker_thread_count {
            let core_id = core_ids[id % core_ids.len()];
            thread::spawn({
                if cfg.cpu_thread_pinning {
                    core_affinity::set_for_current(core_id);
                }
                create_worker_task(
                    rx_read_replies_cpu.clone(),
                    tx_empty_buffers.clone(),
                    tx_nonce_data.clone(),
                    None,
                )
            });
        }

        for _ in 0..gpu_worker_thread_count {
            thread::spawn({
                let context =
                    GpuContext::new(cfg.gpu_platform, cfg.gpu_device, cfg.gpu_nonces_per_cache);
                create_worker_task(
                    rx_read_replies_gpu.clone(),
                    tx_empty_buffers.clone(),
                    tx_nonce_data.clone(),
                    Some(context),
                )
            });
        }

        let core = Core::new().unwrap();
        Miner {
            reader_task_count: drive_id_to_plots.len(),
            reader: Reader::new(
                drive_id_to_plots,
                reader_thread_count,
                rx_empty_buffers,
                tx_read_replies_cpu,
                tx_read_replies_gpu,
            ),
            rx_nonce_data,
            account_id: cfg.account_id,
            target_deadline: cfg.target_deadline,
            request_handler: RequestHandler::new(
                cfg.url,
                &cfg.secret_phrase,
                cfg.timeout,
                core.handle(),
            ),
            state: Arc::new(Mutex::new(State {
                height: 0,
                best_deadline: u64::MAX,
                base_target: 1,
                processed_reader_tasks: 0,
                sw: Stopwatch::new(),
                scanning: false,
            })),
            get_mining_info_interval: cfg.get_mining_info_interval,
            core,
            wakeup_after: cfg.hdd_wakeup_after * 1000, // ms -> s
        }
    }

    pub fn run(mut self) {
        let handle = self.core.handle();
        let request_handler = self.request_handler.clone();

        // you left me no choice!!! at least not one that I could have worked out in two weeks...
        let reader = Rc::new(RefCell::new(self.reader));

        let state = self.state.clone();
        // there might be a way to solve this without two nested moves
        let get_mining_info_interval = self.get_mining_info_interval;
        let wakeup_after = self.wakeup_after;
        handle.spawn(
            Interval::new(
                Instant::now(),
                Duration::from_millis(get_mining_info_interval),
            ).for_each(move |_| {
                let state = state.clone();
                let reader = reader.clone();
                request_handler.get_mining_info().then(move |mining_info| {
                    match mining_info {
                        Ok(mining_info) => {
                            let mut state = state.lock().unwrap();
                            if mining_info.height > state.height {
                                state.best_deadline = u64::MAX;
                                state.height = mining_info.height;
                                state.base_target = mining_info.base_target;

                                let gensig =
                                    burstmath::decode_gensig(&mining_info.generation_signature);
                                let scoop = burstmath::calculate_scoop(mining_info.height, &gensig);

                                info!("new block: height={}, scoop={}", mining_info.height, scoop);

                                reader.borrow_mut().start_reading(
                                    mining_info.height,
                                    scoop,
                                    &Arc::new(gensig),
                                );
                                state.sw.restart();
                                state.processed_reader_tasks = 0;
                                state.scanning = true;
                            } else if !state.scanning
                                && wakeup_after != 0
                                && state.sw.elapsed_ms() > wakeup_after
                            {
                                info!("HDD, wakeup!");
                                reader.borrow_mut().wakeup();
                                state.sw.restart();
                            }
                        }
                        _ => warn!("error getting mining info"),
                    }
                    future::ok(())
                })
            }).map_err(|e| panic!("interval errored: err={:?}", e)),
        );

        let account_id = self.account_id;
        let target_deadline = self.target_deadline;
        let request_handler = self.request_handler.clone();
        let inner_handle = handle.clone();
        let state = self.state.clone();
        let reader_task_count = self.reader_task_count;
        handle.spawn(
            self.rx_nonce_data
                .for_each(move |nonce_data| {
                    let mut state = state.lock().unwrap();
                    let deadline = nonce_data.deadline / state.base_target;
                    if state.best_deadline > deadline && deadline < target_deadline {
                        state.best_deadline = deadline;
                        request_handler.submit_nonce(
                            &inner_handle,
                            account_id,
                            nonce_data.nonce,
                            nonce_data.height,
                            deadline,
                            0,
                        );
                        info!(
                            "deadline found: nonce={}, deadline={}",
                            nonce_data.nonce, deadline
                        );
                    }
                    if nonce_data.reader_task_processed {
                        state.processed_reader_tasks += 1;
                        if state.processed_reader_tasks == reader_task_count {
                            info!("round finished: roundtime={}ms", state.sw.elapsed_ms());
                            state.sw.restart();
                            state.scanning = false;
                        }
                    }
                    Ok(())
                }).map_err(|e| panic!("interval errored: err={:?}", e)),
        );

        self.core.run(future::empty::<(), ()>()).unwrap();
    }
}

#[cfg(test)]
mod test {

    #[test]
    fn test_new_miner() {}
}
