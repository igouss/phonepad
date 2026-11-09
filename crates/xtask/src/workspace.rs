use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Package {
    pub name: String,
    pub version: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Member {
    pub package: Package,
    pub manifest: String,
    pub dependencies: BTreeSet<Package>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Workspace {
    pub members: Vec<Member>,
}

impl Workspace {
    pub fn member(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.package.name == name)
    }

    pub fn is_member(&self, package: &Package) -> bool {
        self.members.iter().any(|m| &m.package == package)
    }
}

pub fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| String::from("cargo"))
}

pub fn load(root: &Path) -> Result<Workspace, String> {
    let manifest: PathBuf = root.join("Cargo.toml");
    let output: Output = Command::new(cargo())
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(&manifest)
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("cargo metadata is not JSON: {e}"))?;
    parse(&metadata)
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("cargo metadata: missing string `{key}`"))
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("cargo metadata: missing array `{key}`"))
}

fn is_normal(dep: &Value) -> Result<bool, String> {
    Ok(array(dep, "dep_kinds")?
        .iter()
        .any(|kind| kind.get("kind").is_some_and(Value::is_null)))
}

pub fn parse(metadata: &Value) -> Result<Workspace, String> {
    let root: &Path = Path::new(text(metadata, "workspace_root")?);
    let mut packages: BTreeMap<&str, (Package, &str)> = BTreeMap::new();
    for package in array(metadata, "packages")? {
        let identity: Package = Package {
            name: text(package, "name")?.to_string(),
            version: text(package, "version")?.to_string(),
        };
        packages.insert(
            text(package, "id")?,
            (identity, text(package, "manifest_path")?),
        );
    }
    let resolve: &Value = metadata
        .get("resolve")
        .ok_or("cargo metadata: missing `resolve`")?;
    let mut dependencies: BTreeMap<&str, BTreeSet<Package>> = BTreeMap::new();
    for node in array(resolve, "nodes")? {
        let mut normal: BTreeSet<Package> = BTreeSet::new();
        for dep in array(node, "deps")? {
            if is_normal(dep)? {
                let target: &str = text(dep, "pkg")?;
                let (package, _): &(Package, &str) = packages
                    .get(target)
                    .ok_or_else(|| format!("cargo metadata: unknown package `{target}`"))?;
                normal.insert(package.clone());
            }
        }
        dependencies.insert(text(node, "id")?, normal);
    }
    let mut members: Vec<Member> = Vec::new();
    for id in array(metadata, "workspace_members")? {
        let id: &str = id
            .as_str()
            .ok_or("cargo metadata: workspace member id is not a string")?;
        let (package, manifest): &(Package, &str) = packages
            .get(id)
            .ok_or_else(|| format!("cargo metadata: unknown member `{id}`"))?;
        let relative: &Path = Path::new(manifest)
            .strip_prefix(root)
            .map_err(|e| format!("{manifest}: {e}"))?;
        members.push(Member {
            package: package.clone(),
            manifest: relative.to_string_lossy().into_owned(),
            dependencies: dependencies.remove(id).unwrap_or_default(),
        });
    }
    members.sort_by(|a, b| a.package.cmp(&b.package));
    Ok(Workspace { members })
}

#[cfg(test)]
mod tests {
    use super::{Package, Workspace, parse};
    use serde_json::{Value, json};
    use std::collections::BTreeSet;

    fn metadata(core_dep_kinds: Value) -> Value {
        json!({
            "workspace_root": "/w",
            "workspace_members": ["app-id", "core-id"],
            "packages": [
                {"name": "app", "version": "0.1.0", "id": "app-id", "manifest_path": "/w/crates/app/Cargo.toml"},
                {"name": "core", "version": "0.1.0", "id": "core-id", "manifest_path": "/w/crates/core/Cargo.toml"},
                {"name": "libc", "version": "0.2.1", "id": "libc-id", "manifest_path": "/r/libc/Cargo.toml"}
            ],
            "resolve": {"nodes": [
                {"id": "app-id", "deps": [{"name": "core", "pkg": "core-id", "dep_kinds": [{"kind": null, "target": null}]}]},
                {"id": "core-id", "deps": [{"name": "libc", "pkg": "libc-id", "dep_kinds": core_dep_kinds}]},
                {"id": "libc-id", "deps": []}
            ]}
        })
    }

    fn libc() -> Package {
        Package {
            name: String::from("libc"),
            version: String::from("0.2.1"),
        }
    }

    #[test]
    fn dev_and_build_dependencies_are_not_normal() {
        let workspace: Workspace = parse(&metadata(
            json!([{"kind": "dev", "target": null}, {"kind": "build", "target": null}]),
        ))
        .unwrap();
        assert_eq!(
            workspace.member("core").unwrap().dependencies,
            BTreeSet::new()
        );
    }

    #[test]
    fn a_normal_dependency_is_reported_with_its_version() {
        let workspace: Workspace =
            parse(&metadata(json!([{"kind": null, "target": null}]))).unwrap();
        assert_eq!(
            workspace.member("core").unwrap().dependencies,
            BTreeSet::from([libc()])
        );
    }

    #[test]
    fn manifests_are_relative_to_the_workspace_root() {
        let workspace: Workspace = parse(&metadata(json!([]))).unwrap();
        assert_eq!(
            workspace.member("app").unwrap().manifest,
            "crates/app/Cargo.toml"
        );
    }

    #[test]
    fn non_members_are_not_members() {
        let workspace: Workspace = parse(&metadata(json!([]))).unwrap();
        assert_eq!(
            (workspace.member("libc"), workspace.is_member(&libc())),
            (None, false)
        );
    }
}
