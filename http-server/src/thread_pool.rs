use std::sync::mpsc;
use std::thread;


struct MyTask<TcpStream> {
    sender: mpsc::Sender<TcpStream>,
    task: thread::JoinHandle<()>,
}
pub struct ThreadPool<T> {
    tasks: Vec<Box<MyTask<T>>>,
    index: u16,
    max: u16,
}
impl<TcpStream:'static + Send,> ThreadPool<TcpStream> {
    pub(super) fn new<F: FnMut(TcpStream) + Clone + Send + 'static>(num: u16, f: F) -> Self {
        let mut r = Self {
            tasks: Vec::new(),
            index: 0,
            max: num,
        };
        for _ in 0..num {
            let (tx, rx) = mpsc::channel();
            let mut f = f.clone();
            r.tasks.push(Box::new(MyTask {
                sender: tx,
                task: thread::spawn(move || {
                    for stream in rx {
                        f(stream);
                    }
                    //    loop{
                    // 	 let r = rx.recv();
                    // 	 match r{
                    // 		Ok(stream)=>{
                    // 			f(stream);
                    // 		},
                    // 		Err(e)=>{}
                    // 	 }
                    //    }
                }),
            }))
        }
        r
    }

    pub(super) fn poll(&mut self, stream: TcpStream) {
        if self.index >= self.max {
            self.index = 0;
        }
        //println!("current:{}", self.index);
        if let Some(task) = self.tasks.get(self.index as usize) {
            match task.sender.send(stream){
                Ok(_) => {
					self.index += 1;
				},
                Err(e) => {
                  println!("dispatch stream error:{}",e.to_string());
				},
            }
        }
    }

    pub(super) fn join(self) {
        for task in self.tasks {
            let _r = task.task.join();
        }
    }
}
