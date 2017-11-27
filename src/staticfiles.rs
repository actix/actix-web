//! Static files support.
//!
//! TODO: needs to re-implement actual files handling, current impl blocks
use std::io;
use std::io::Read;
use std::fmt::Write;
use std::fs::{File, DirEntry};
use std::path::PathBuf;

use task::Task;
use route::RouteHandler;
use mime_guess::get_mime_type;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use httpcodes::{HTTPOk, HTTPNotFound, HTTPForbidden};

/// Static files handling
///
/// Can be registered with `Application::route_handler()`.
///
/// ```rust
/// extern crate actix_web;
/// use actix_web::*;
///
/// fn main() {
///     let app = Application::default("/")
///         .route_handler("/static", StaticFiles::new(".", true))
///         .finish();
/// }
/// ```
pub struct StaticFiles {
    directory: PathBuf,
    accessible: bool,
    _show_index: bool,
    _chunk_size: usize,
    _follow_symlinks: bool,
    prefix: String,
}

impl StaticFiles {
    /// Create new `StaticFiles` instance
    ///
    /// `dir` - base directory
    /// `index` - show index for directory
    pub fn new<D: Into<PathBuf>>(dir: D, index: bool) -> StaticFiles {
        let dir = dir.into();

        let (dir, access) = match dir.canonicalize() {
            Ok(dir) => {
                if dir.is_dir() {
                    (dir, true)
                } else {
                    warn!("Is not directory `{:?}`", dir);
                    (dir, false)
                }
            },
            Err(err) => {
                warn!("Static files directory `{:?}` error: {}", dir, err);
                (dir, false)
            }
        };

        StaticFiles {
            directory: dir,
            accessible: access,
            _show_index: index,
            _chunk_size: 0,
            _follow_symlinks: false,
            prefix: String::new(),
        }
    }

    fn index(&self, relpath: &str, filename: &PathBuf) -> Result<HttpResponse, io::Error> {
        let index_of = format!("Index of {}/{}", self.prefix, relpath);
        let mut body = String::new();

        for entry in filename.read_dir()? {
            if self.can_list(&entry) {
                let entry = entry.unwrap();
                // show file url as relative to static path
                let file_url = format!(
                    "{}/{}", self.prefix,
                    entry.path().strip_prefix(&self.directory).unwrap().to_string_lossy());

                // if file is a directory, add '/' to the end of the name
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_dir() {
                        //format!("<li><a href=\"{}\">{}</a></li>", file_url, file_name));
                        let _ = write!(body, "<li><a href=\"{}\">{}/</a></li>",
                                       file_url, entry.file_name().to_string_lossy());
                    } else {
                        // write!(body, "{}/", entry.file_name())
                        let _ = write!(body, "<li><a href=\"{}\">{}</a></li>",
                                       file_url, entry.file_name().to_string_lossy());
                    }
                } else {
                    continue
                }
            }
        }

        let html = format!("<html>\
                            <head><title>{}</title></head>\
                            <body><h1>{}</h1>\
                            <ul>\
                            {}\
                            </ul></body>\n</html>", index_of, index_of, body);
        Ok(
            HTTPOk.build()
                .content_type("text/html; charset=utf-8")
                .body(html).unwrap()
        )
    }

    fn can_list(&self, entry: &io::Result<DirEntry>) -> bool {
        if let Ok(ref entry) = *entry {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with('.') {
                    return false
                }
            }
            if let Ok(ref md) = entry.metadata() {
                let ft = md.file_type();
                return ft.is_dir() || ft.is_file() || ft.is_symlink()
            }
        }
        false
    }
}

impl<S: 'static> RouteHandler<S> for StaticFiles {

    fn set_prefix(&mut self, prefix: String) {
        if prefix != "/" {
            self.prefix += &prefix;
        }
    }

    fn handle(&self, req: HttpRequest<S>) -> Task {
        if !self.accessible {
            Task::reply(HTTPNotFound)
        } else {
            let mut hidden = false;
            let filepath = req.path()[self.prefix.len()..]
                .split('/').filter(|s| {
                    if s.starts_with('.') {
                        hidden = true;
                    }
                    !s.is_empty()
                })
                .fold(String::new(), |s, i| {s + "/" + i});

            // hidden file
            if hidden {
                return Task::reply(HTTPNotFound)
            }

            // full filepath
            let idx = if filepath.starts_with('/') { 1 } else { 0 };
            let filename = match self.directory.join(&filepath[idx..]).canonicalize() {
                Ok(fname) => fname,
                Err(err) => return match err.kind() {
                    io::ErrorKind::NotFound => Task::reply(HTTPNotFound),
                    io::ErrorKind::PermissionDenied => Task::reply(HTTPForbidden),
                    _ => Task::error(err),
                }
            };

            if filename.is_dir() {
                match self.index(&filepath[idx..], &filename) {
                    Ok(resp) => Task::reply(resp),
                    Err(err) => match err.kind() {
                        io::ErrorKind::NotFound => Task::reply(HTTPNotFound),
                        io::ErrorKind::PermissionDenied => Task::reply(HTTPForbidden),
                        _ => Task::error(err),
                    }
                }
            } else {
                let mut resp = HTTPOk.build();
                if let Some(ext) = filename.extension() {
                    let mime = get_mime_type(&ext.to_string_lossy());
                    resp.content_type(format!("{}", mime).as_str());
                }
                match File::open(filename) {
                    Ok(mut file) => {
                        let mut data = Vec::new();
                        let _ = file.read_to_end(&mut data);
                        Task::reply(resp.body(data).unwrap())
                    },
                    Err(err) => Task::error(err),
                }
            }
        }
    }
}
