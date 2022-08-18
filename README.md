# rust-xfinal
A safe and performance web server framework that is written by Rust.

### Introduction
This is the beginning of the aim to write a safe web server framework by Rust. For now, this repository has not provided a complete and stable function, as [xfinal](https://github.com/xmh0511/xfinal) has done, written by modern c++. The aim is to build up a web server framework, which has the same functionalities as `xfinal` has.

### Advantages 
1. Since the advantages of Rust are that it is safe on memory and multiple threads, and it is a modern programming language that has almost no pieces baggage c++ has, the framework that is written based on Rust has no worry about the hard problems with memory and data race. Rust can guarantee the safety of these aspects. 

2. Moreover, Rust has the characteristics of zero-cost abstraction, as c++ has, hence the performance of the framework will be desired as you expect, rapid! 

3. Rust has a very convenient package manager: Cargo, which can make you feel free to use any third crate without being necessary to use CMake to manage these dependencies or even to write obscure rules that are ugly. 


### Features that have been supported
1. Register router.
2. Register middleware.
3. Query text-plain body, url-form data, and multiple-form data(include uploaded files).
4. Chunked transfer
5. Accept Range requests, which implies that rust-xfinal has supported to download file with the resume breakpoint.

### Features that haven't been supported yet
1. Cookie/Session
2. View-engine

### Usage
> 1. HTTP "hello, world"
````rust
use http_server::{
    end_point, EndPoint, HttpServer, Request, Response, GET,
};
fn main(){
   let mut http_server = HttpServer::create(end_point!(0.0.0.0:8080), 10);
   http_server.route(GET, "/").reg(|req: &Request, res: &mut Response| {
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
   fn interrupt_second(req:& Request,res:&mut Response) ->bool{
        println!("invoke middleware2");
        match req.get_param("id") {
            Some(v) => {
                if v == "1"{
                    true
                }else{
                    res.write_string("invalid request, invalid id value", 400);
                    false
                }
            },
            None => {
                res.write_string("invalid request, no id", 400);
                false
            },
        }
   }
   let middlewares = inject_middlewares! {
      |req:& Request,res:&mut Response|->bool{
          println!("invoke middleware");
          true
      },interrupt_second //, middleware 2,..., so forth
   };
   
   http_server.route(GET, "/middle").reg_with_middlewares(
        middlewares,
        |req: &Request, res: &mut Response| {
            println!("invoke router");
            res.write_string(String::from("hello from router"), 200);
        },
   );
   http_server.run();
}
````

> 3. Query information from Request
````rust
http_server.route(GET, "/query").reg(|req: &Request, res: &mut Response| {
    let r:Option<&str> = req.get_query("id");
    let host = req.get_header("Host");
    let file = req.get_file("file");
    let version = req.get_version();
    let method = req.get_method();
    let headers = req.get_headers();
    let forms = req.get_queries();
    let get_all_files = req.get_files();
    let url = req.get_url();
    let id = req.get_param("id"); // /a?id=0
    res.write_string("ok", 200);
});
````
> 4. Chunked transfer and/or Rangeable
````rust
http_server.route([GET,HEAD], "/query").reg(|_req: &Request, res: &mut Response| {
    // file: res.write_file("./upload/test.mp4",200).chunked().enable_range();
   //string: res.write_string("abcdefg",200).chunked();
  // res.write_string("abcdefg",200).enable_range();
  // file: res.write_file("./upload/test.mp4",200).enable_range().chunked();
  // No sequence requirement, whatever combination as you go.
});
````
>5. Wildcard path
````rust
http_server.route(GET, "/wildcard/*").reg(|_req: &Request, res: &mut Response|{
      let s = format!("hello from {}", req.get_url());
      res.write_string(&s, 200);
});
````


