#![allow(dead_code, unused_variables)]
use std::rc::Rc;

use task::Task;
use route::RouteHandler;
use payload::Payload;
use httpcodes::HTTPOk;
use httprequest::HttpRequest;


pub struct StaticFiles {
    directory: String,
    show_index: bool,
    chunk_size: usize,
    follow_synlinks: bool,
}

impl<S: 'static> RouteHandler<S> for StaticFiles {

    fn handle(&self, req: HttpRequest, payload: Payload, state: Rc<S>) -> Task {
        Task::reply(HTTPOk)
    }
}
