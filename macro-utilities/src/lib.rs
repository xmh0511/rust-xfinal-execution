use std::{str::FromStr};

use proc_macro::TokenStream;
#[proc_macro]
pub fn end_point(input:TokenStream)->TokenStream{
   let str = input.to_string();
   let split = str.split(":");
   let size = split.clone().count();
   if size!=2{
	  panic!("invalid endpoint value!")
   }else{
	  let vec:Vec<&str> = split.collect();
	  let addr = vec[0];
	  let port = vec[1];
	  let addr:Vec<&str> = addr.split(".").collect();
	  let addr = addr.join(",");
	  let mut value = format!("port:{},ip_address:[{}]",port,addr);
	  value  = "EndPoint{".to_string() + &value;
	  value = value + "}";
	  TokenStream::from_str(&value).unwrap()
   }
}
