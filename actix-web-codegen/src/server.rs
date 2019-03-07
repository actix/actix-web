use std::collections::HashSet;
use std::env;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn;

/// Thrift mux server impl
pub struct Server {}

impl Server {
    fn new() -> Server {
        Server {}
    }

    /// generate servers
    pub fn generate(input: TokenStream) {
        let mut srv = Server::new();
        let ast: syn::ItemFn = syn::parse2(input).unwrap();
        println!("T: {:?}", ast.ident);

        // quote! {
        //     #ast

        //     #(#servers)*
        // }
    }
}
