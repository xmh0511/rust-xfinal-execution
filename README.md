# rust-xfinal
A safet web server framework that is written by Rust.

### Introduction
This is the beginning of the aim to write a safe web server framework by Rust. For now, this repository has not provided a complete and stable function, as [xfinal](https://github.com/xmh0511/xfinal) has done, written by modern c++. The aim is to build up a web server framework, which has the same functionalities as `xfinal` has.

### Advantages 
1. Since the advantages of Rust are that it is safe on memory and multiple threads, and it is a modern programming language that has almost no pieces baggage c++ has, the framework that is written based on Rust has no worry about the hard problems with memory and data race. Rust can guarantee the safety of these aspects. 

2. Moreover, Rust has the characteristics of zero-cost abstraction, as c++ has, hence the performance of the framework will be desired as you expect, rapid! 

3. Rust has a very convenient package manager: Cargo, which can make you feel free to use any third crate without being necessary to use CMake to manage these dependencies or even to write obscure rules that are ugly. 



### Usage
> 1. HTTP "hello, world"
````rust
use http_server::{
    end_point, EndPoint, HttpServer, Request, Response, GET,
};
fn main(){
   let mut http_server = HttpServer::create(end_point!(0.0.0.0:8080), 10);
   http_server.route::<GET>("/").reg(|req: &Request, res: &mut Response| {
       res.write_string(String::from("hello, world"), 200);
   });
   http_server.run();
}
````

> 2. Use middlewares
````rust
use http_server::{
    end_point, inject_middlewares, EndPoint, HttpServer, MiddleWare, Request, Response, GET,
};
fn main(){
   let mut http_server = HttpServer::create(end_point!(0.0.0.0:8080), 10);
   let middlewares = inject_middlewares! {
      |req:& Request,res:&mut Response|->bool{
          println!("invoke middleware");
          true
      } //, middleware 2,..., so forth
   };
   
   http_server.route::<GET>("/middle").reg_with_middlewares(
        middlewares,
        |req: &Request, res: &mut Response| {
            println!("invoke router");
            res.write_string(String::from("hello from router"), 200);
        },
   );
   http_server.run();
}
````



