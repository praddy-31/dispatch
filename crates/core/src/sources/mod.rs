//! Source adapters: each turns a downloaded document into `Item`s and never touches the
//! store. More adapters (arXiv API, OpenAlex, Semantic Scholar) join here later.

pub mod arxiv;
pub mod arxiv_api;
pub mod rss;
