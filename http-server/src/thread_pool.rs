use std::sync::mpsc::{self, SendError, Sender};
use std::thread;


struct MyTask<T> {
    task: thread::JoinHandle<()>,
	sender:Sender<T>
}
pub struct ThreadPool<T> {
    tasks: Vec<Box<MyTask<T>>>,
	index:u16,
	max:u16
}
impl<T: 'static + Send> ThreadPool<T> {
    pub(super) fn new<F: FnMut(T) + Clone + Send + 'static>(num: u16, f: F) -> Self {
        let mut r = Self {
            tasks: Vec::new(),
			index:0,
			max:num
        };
        for _ in 0..num {
            let mut f = f.clone();
			let (tx, rx) = mpsc::channel();
            r.tasks.push(Box::new(MyTask {
				sender:tx,
                task: thread::spawn(move || {
                    loop {
                        let r = rx.recv();
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
        if self.index >= self.max {
            self.index = 0;
        }
        //println!("current:{}", self.index);
        if let Some(task) = self.tasks.get(self.index as usize) {
            match task.sender.send(data) {
                Ok(_) => {
                    self.index += 1;
                    return Ok(());
                }
                Err(e) => {
                    //println!("dispatch stream error:{}",e.to_string());
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    pub(super) fn join(self) {
        for task in self.tasks {
            let _r = task.task.join();
        }
    }
}
