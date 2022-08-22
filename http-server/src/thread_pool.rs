use std::sync::mpsc::{self, SendError, Sender};
use std::thread;
use std::sync::{Arc,Mutex};

struct MyTask {
    task: thread::JoinHandle<()>,
}
pub struct ThreadPool<T> {
    tasks: Vec<Box<MyTask>>,
	sender:Sender<T>
}
impl<T: 'static + Send> ThreadPool<T> {
    pub(super) fn new<F: FnMut(T) + Clone + Send + 'static>(num: u16, f: F) -> Self {
		let (tx, rx) = mpsc::channel();
        let mut r = Self {
            tasks: Vec::new(),
			sender:tx
        };
		let receiver = Arc::new(Mutex::new(rx));
        for _ in 0..num {
            let mut f = f.clone();
			let rx = Arc::clone(&receiver);
            r.tasks.push(Box::new(MyTask {
                task: thread::spawn(move || {
                    loop {
                        let r = rx.lock().unwrap().recv();
                        match r {
                            Ok(stream) => {
                                f(stream);
                            }
                            Err(e) => {
								println!("recv() error: {}",e.to_string());
							}
                        }
                    }
                }),
            }))
        }
        r
    }

    pub(super) fn poll(&mut self, data: T) -> Result<(), SendError<T>> {
        //println!("current:{}", self.index);
		match self.sender.send(data) {
			Ok(_) => {
				return Ok(())
			}
			Err(e) => {
				return Err(e);
			}
		}
    }

    pub(super) fn join(self) {
        for task in self.tasks {
            let _r = task.task.join();
        }
    }
}
