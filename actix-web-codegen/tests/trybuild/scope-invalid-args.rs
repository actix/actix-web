use actix_web_codegen::scope;

const PATH: &str = "/api";

#[scope(PATH)]
mod api_const {}

#[scope(true)]
mod api_bool {}

#[scope(123)]
mod api_num {}

fn main() {}
