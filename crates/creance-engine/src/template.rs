use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
};

use creance_sandbox::canonicalize_rule_path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub pkg_dir: PathBuf,
    pub project_root: PathBuf,
    pub store: PathBuf,
    pub home: PathBuf,
    pub cache: PathBuf,
    pub run_tmp: PathBuf,
}

impl Context {
    pub fn from_roots(
        pkg_dir: impl Into<PathBuf>,
        project_root: impl Into<PathBuf>,
        store: impl Into<PathBuf>,
        home: impl Into<PathBuf>,
        cache: impl Into<PathBuf>,
        run_tmp: impl Into<PathBuf>,
    ) -> Self {
        Self {
            pkg_dir: pkg_dir.into(),
            project_root: project_root.into(),
            store: store.into(),
            home: home.into(),
            cache: cache.into(),
            run_tmp: run_tmp.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Templater {
    ctx: Context,
}

impl Templater {
    pub fn new(ctx: Context) -> Self {
        Self { ctx }
    }

    pub fn templatize(&self, abs: impl AsRef<Path>) -> String {
        let path = canonicalize_rule_path(abs.as_ref());
        let mut roots = self.variable_roots();
        roots.sort_by_key(|root| Reverse(root.1.as_os_str().len()));

        for (name, root) in roots {
            let root = canonicalize_rule_path(root);
            if path == root {
                return format!("${{{name}}}");
            }
            if let Ok(rest) = path.strip_prefix(&root) {
                let rest = path_to_slash(rest);
                return format!("${{{name}}}/{rest}");
            }
        }

        path.to_string_lossy().into_owned()
    }

    pub fn resolve(&self, template: &str) -> PathBuf {
        let template = template.strip_suffix("/**").unwrap_or(template);
        for (name, root) in self.variable_roots() {
            let marker = format!("${{{name}}}");
            if template == marker {
                return root;
            }
            if let Some(rest) = template.strip_prefix(&(marker + "/")) {
                return root.join(rest);
            }
        }
        PathBuf::from(template)
    }

    pub fn resolve_many(&self, templates: &[String]) -> Vec<PathBuf> {
        templates
            .iter()
            .map(|template| self.resolve(template))
            .collect()
    }

    fn variable_roots(&self) -> Vec<(&'static str, PathBuf)> {
        vec![
            ("PKG_DIR", self.ctx.pkg_dir.clone()),
            ("PROJECT_ROOT", self.ctx.project_root.clone()),
            ("STORE", self.ctx.store.clone()),
            ("HOME", self.ctx.home.clone()),
            ("CACHE", self.ctx.cache.clone()),
            ("RUN_TMP", self.ctx.run_tmp.clone()),
        ]
    }
}

pub fn generalize(paths: &[PathBuf], ctx: &Context) -> Vec<String> {
    let templater = Templater::new(ctx.clone());
    let mut grouped: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut ungrouped = BTreeSet::new();

    for path in paths {
        let path = canonicalize_rule_path(path);
        if is_runtime_path(&path) {
            continue;
        }

        let templated = templater.templatize(&path);
        if let Some((root, rest)) = split_template_root(&templated) {
            grouped.entry(root).or_default().insert(rest.to_string());
        } else {
            ungrouped.insert(templated);
        }
    }

    let mut out = ungrouped;
    for (root, rest) in grouped {
        if root == "${RUN_TMP}" || rest.len() >= 8 {
            out.insert(format!("{root}/**"));
            continue;
        }

        for item in rest {
            if matches!(root.as_str(), "${CACHE}" | "${STORE}" | "${HOME}")
                && item.split('/').count() > 1
            {
                let first = item.split('/').next().unwrap();
                out.insert(format!("{root}/{first}/**"));
            } else {
                out.insert(format!("{root}/{item}"));
            }
        }
    }

    out.into_iter().collect()
}

fn split_template_root(value: &str) -> Option<(String, &str)> {
    let rest = value.strip_prefix("${")?;
    let (name, path) = rest.split_once("}/")?;
    Some((format!("${{{name}}}"), path))
}

fn path_to_slash(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn is_runtime_path(path: &Path) -> bool {
    runtime_roots().iter().any(|root| path.starts_with(root))
}

fn runtime_roots() -> Vec<PathBuf> {
    let mut roots = [
        "/usr",
        "/System",
        "/Library",
        "/bin",
        "/sbin",
        "/private/etc",
    ]
    .into_iter()
    .map(canonicalize_rule_path)
    .collect::<Vec<_>>();

    if let Some(node) = node_binary_path() {
        let node = canonicalize_rule_path(node);
        if let Some(bin) = node.parent() {
            roots.push(bin.to_path_buf());
            if let Some(root) = bin.parent() {
                roots.push(root.to_path_buf());
            }
        }
    }

    roots
}

fn node_binary_path() -> Option<PathBuf> {
    let output = Command::new("mise")
        .arg("which")
        .arg("node")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .or_else(|| {
            Command::new("/bin/sh")
                .arg("-lc")
                .arg("command -v node")
                .output()
                .ok()
                .filter(|output| output.status.success())
        })?;
    Some(PathBuf::from(String::from_utf8(output.stdout).ok()?.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_and_resolves_each_root_with_longest_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = test_context(temp.path());
        let templater = Templater::new(ctx.clone());

        let pkg_file = ctx.pkg_dir.join("src/index.js");
        assert_eq!(templater.templatize(&pkg_file), "${PKG_DIR}/src/index.js");
        assert_eq!(templater.resolve("${PKG_DIR}/src/index.js"), pkg_file);

        let project_file = ctx.project_root.join("README.md");
        assert_eq!(
            templater.templatize(&project_file),
            "${PROJECT_ROOT}/README.md"
        );
        assert_eq!(templater.resolve("${PROJECT_ROOT}/README.md"), project_file);

        let outside = temp.path().join("outside");
        assert_eq!(
            templater.templatize(&outside),
            canonicalize_rule_path(outside).to_string_lossy()
        );
    }

    #[test]
    fn generalizes_many_package_paths_and_drops_runtime_reads() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = test_context(temp.path());
        std::fs::create_dir_all(&ctx.pkg_dir).unwrap();

        let mut paths = (0..50)
            .map(|idx| {
                let path = ctx.pkg_dir.join(format!("file-{idx}.js"));
                std::fs::write(&path, "").unwrap();
                path
            })
            .collect::<Vec<_>>();
        paths.push(PathBuf::from("/usr/lib/libSystem.B.dylib"));

        assert_eq!(generalize(&paths, &ctx), vec!["${PKG_DIR}/**"]);
    }

    #[test]
    fn generalizes_cache_subtrees_without_collapsing_package_singletons() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = test_context(temp.path());
        let cache_file = ctx.cache.join("node-gyp/headers.tar.gz");
        std::fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
        std::fs::write(&cache_file, "").unwrap();

        assert_eq!(
            generalize(&[cache_file], &ctx),
            vec!["${CACHE}/node-gyp/**"]
        );
    }

    fn test_context(root: &Path) -> Context {
        let project = root.join("project");
        Context::from_roots(
            project.join("node_modules/.pnpm/pkg"),
            project.clone(),
            root.join("store"),
            root.join("home"),
            root.join("home/Library/Caches"),
            root.join("run"),
        )
    }
}
