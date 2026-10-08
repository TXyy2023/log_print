use anyhow::{Context, Result};
use scraper::{Html, Selector};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};
struct Page {
    ids: HashSet<String>,
    links: Vec<String>,
}
fn collect(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in fs::read_dir(path)? {
        let p = e?.path();
        if p.is_dir() {
            collect(&p, out)?
        } else if p.extension().is_some_and(|x| x == "html") {
            out.push(p)
        }
    }
    Ok(())
}
fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            _ => out.push(c),
        }
    }
    out
}
fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}
pub fn verify(directory: &Path, base: &str) -> Result<bool> {
    anyhow::ensure!(
        base.starts_with('/')
            && base.ends_with('/')
            && !base.starts_with("//")
            && !base.split('/').any(|p| p == "." || p == "..")
            && !base.contains(['?', '#', '\\']),
        "--base must be an absolute site path ending in /"
    );
    let root = directory
        .canonicalize()
        .context("built site directory does not exist")?;
    let mut files = vec![];
    collect(&root, &mut files)?;
    anyhow::ensure!(!files.is_empty(), "no HTML pages under {}", root.display());
    let selector = Selector::parse("[id], a[href], img[src], script[src], link[href]").unwrap();
    let mut pages = HashMap::new();
    for p in files {
        let html = Html::parse_document(&fs::read_to_string(&p)?);
        let mut page = Page {
            ids: HashSet::new(),
            links: vec![],
        };
        for el in html.select(&selector) {
            if let Some(id) = el.value().attr("id") {
                page.ids.insert(id.to_owned());
            }
            let attr = match el.value().name() {
                "a" | "link" => "href",
                "img" | "script" => "src",
                _ => continue,
            };
            if let Some(link) = el.value().attr(attr) {
                page.links.push(link.to_owned())
            }
        }
        pages.insert(p, page);
    }
    let mut errors = vec![];
    let mut checked = 0;
    for (file, page) in &pages {
        for link in &page.links {
            if link.starts_with("//") || url::Url::parse(link).is_ok() {
                continue;
            }
            let (path, fragment) = link
                .split_once('#')
                .map(|(a, b)| (a, Some(b)))
                .unwrap_or((link, None));
            let path = decode(path.split('?').next().unwrap());
            let mut reason = None;
            let dst = if path.starts_with('/') {
                if base != "/" && !path.starts_with(base) {
                    reason = Some("outside site base");
                    root.clone()
                } else {
                    root.join(&path[base.len()..])
                }
            } else if path.is_empty() {
                file.clone()
            } else {
                file.parent().unwrap().join(path)
            };
            let mut dst = clean(&dst);
            if !dst.starts_with(&root) {
                reason = Some("outside site directory");
            }
            if reason.is_none() {
                if dst.is_dir() {
                    dst = dst.join("index.html");
                }
                if !dst.exists() && dst.extension().is_none() {
                    dst.set_extension("html");
                }
                checked += 1;
                if !dst.exists() {
                    reason = Some("missing file")
                } else if !dst.canonicalize()?.starts_with(&root) {
                    reason = Some("outside site directory")
                } else if let (Some(fragment), Some(target)) = (fragment, pages.get(&dst)) {
                    if !fragment.is_empty() && !target.ids.contains(&decode(fragment)) {
                        reason = Some("missing anchor");
                    }
                }
            }
            if let Some(reason) = reason {
                errors.push(json!(
                    { "page" : file.strip_prefix(& root) ?, "link" : link,
                    "reason" : reason }
                ));
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({ "pages" : pages.len(),
        "local_targets_checked" : checked, "errors" : errors }))?
    );
    Ok(errors.is_empty())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn links_anchors_bases_and_escape() {
        let td = tempfile::tempdir().unwrap();
        fs::write(
            td.path().join("index.html"),
            "<a href='/log_print/other#%E4%B8%AD'>ok</a>",
        )
        .unwrap();
        fs::write(td.path().join("other.html"), "<h1 id='中'>title</h1>").unwrap();
        assert!(verify(td.path(), "/log_print/").unwrap());
        assert!(!verify(td.path(), "/").unwrap());
        fs::write(
            td.path().join("index.html"),
            "<a href='../escape'>bad</a><a href='other#missing'>bad</a>",
        )
        .unwrap();
        assert!(!verify(td.path(), "/").unwrap());
        assert!(verify(td.path(), "/../").is_err());
    }
}
