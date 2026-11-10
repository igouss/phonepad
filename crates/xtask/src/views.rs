use crate::machine::{Labels, Machine, Transition, explore};
use crate::mermaid::{Ids, escape_label};
use crate::workspace::{Member, Package, Workspace};
use phonepad_core::stream::{Stream, StreamInput};
use phonepad_core::{desk, stream, wallpaper};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub const VIEWS_DIR: &str = "docs/views";

pub type Files = BTreeMap<String, String>;

fn stream_phase(state: &Stream) -> String {
    format!("{:?}", stream::phase(state))
}

fn stream_input(input: &StreamInput) -> String {
    match input {
        StreamInput::Visibility(visible) => format!("Visibility({visible})"),
        StreamInput::UnitReady(offer) => {
            format!("UnitReady({:?}, {} B)", offer.kind, offer.unit.bytes)
        }
        StreamInput::Missed => String::from("Missed"),
        StreamInput::Ack { id, .. } => format!("Ack({id})"),
    }
}

pub fn machines() -> BTreeMap<String, Machine> {
    BTreeMap::from([
        (
            String::from("stream"),
            explore(
                stream::all_states(),
                stream::all_inputs(),
                stream::decide,
                stream::evolve,
                Labels {
                    state: stream_phase,
                    input: stream_input,
                },
            ),
        ),
        (
            String::from("desk"),
            explore(
                desk::all_states(),
                desk::all_inputs(),
                desk::decide,
                desk::evolve,
                Labels::debug(),
            ),
        ),
        (
            String::from("wallpaper"),
            explore(
                wallpaper::all_states(),
                wallpaper::all_inputs(),
                wallpaper::decide,
                wallpaper::evolve,
                Labels::debug(),
            ),
        ),
    ])
}

pub fn own_root() -> Result<PathBuf, String> {
    canonical(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

pub fn canonical(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub fn regenerate(root: &Path, out: &Path) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let out: PathBuf = canonical(out)?;
    let root: PathBuf = canonical(root)?;
    if root == own_root()? {
        let workspace: Workspace = crate::workspace::load(&root)?;
        write_files(&out, &render(&workspace, &machines()))
    } else {
        delegate(&root, &out)
    }
}

fn delegate(root: &Path, out: &Path) -> Result<(), String> {
    let output: Output = Command::new(crate::workspace::cargo())
        .current_dir(root)
        .args(["run", "-q", "-p", "xtask", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .args(["--", "views", "--out"])
        .arg(out)
        .output()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "xtask views in {} failed: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn write_files(dir: &Path, files: &Files) -> Result<(), String> {
    for (name, content) in files {
        let path: PathBuf = dir.join(name);
        std::fs::write(&path, content).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

pub fn read_dir_files(dir: &Path) -> Result<Files, String> {
    let mut files: Files = Files::new();
    let entries: std::fs::ReadDir =
        std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let path: PathBuf = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        let name: String = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let content: String =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        files.insert(name, content);
    }
    Ok(files)
}

pub fn render(workspace: &Workspace, machines: &BTreeMap<String, Machine>) -> Files {
    let graph: CrateGraph = CrateGraph::new(workspace);
    let mut files: Files = Files::new();
    files.insert(String::from("crate-graph.mmd"), graph.mermaid());
    let mut machine_json: serde_json::Map<String, Value> = serde_json::Map::new();
    for (name, machine) in machines {
        files.insert(format!("{name}.mmd"), state_diagram(machine));
        machine_json.insert(name.clone(), machine_value(machine));
    }
    let views: Value = json!({ "crates": graph.value(), "machines": machine_json });
    files.insert(String::from("views.json"), format!("{views:#}\n"));
    files
}

struct CrateNode {
    id: String,
    label: String,
    manifest: Option<String>,
}

struct CrateGraph {
    nodes: Vec<CrateNode>,
    edges: BTreeSet<(String, String)>,
}

fn crate_label(workspace: &Workspace, package: &Package) -> String {
    if workspace.is_member(package) {
        package.name.clone()
    } else {
        format!("{} {}", package.name, package.version)
    }
}

impl CrateGraph {
    fn new(workspace: &Workspace) -> CrateGraph {
        let mut ids: Ids = Ids::default();
        let externals: BTreeSet<&Package> = workspace
            .members
            .iter()
            .flat_map(|m: &Member| m.dependencies.iter())
            .filter(|p| !workspace.is_member(p))
            .collect();
        let members: Vec<CrateNode> = workspace
            .members
            .iter()
            .map(|m| CrateNode {
                id: ids.id("c_", &crate_label(workspace, &m.package)),
                label: crate_label(workspace, &m.package),
                manifest: Some(m.manifest.clone()),
            })
            .collect();
        let nodes: Vec<CrateNode> = members
            .into_iter()
            .chain(externals.into_iter().map(|p| CrateNode {
                id: ids.id("c_", &crate_label(workspace, p)),
                label: crate_label(workspace, p),
                manifest: None,
            }))
            .collect();
        let edges: BTreeSet<(String, String)> = workspace
            .members
            .iter()
            .flat_map(|m| m.dependencies.iter().map(move |p| (&m.package, p)))
            .map(|(from, to)| {
                (
                    ids.id("c_", &crate_label(workspace, from)),
                    ids.id("c_", &crate_label(workspace, to)),
                )
            })
            .collect();
        CrateGraph { nodes, edges }
    }

    fn mermaid(&self) -> String {
        let mut out: String = String::from("flowchart LR\n");
        out.push_str("  classDef workspace fill:#2b6cb0,stroke:#1a365d,color:#ffffff\n");
        out.push_str("  classDef external fill:#edf2f7,stroke:#a0aec0,color:#1a202c\n");
        for node in &self.nodes {
            let class: &str = if node.manifest.is_some() {
                "workspace"
            } else {
                "external"
            };
            out.push_str(&format!(
                "  {}[\"{}\"]:::{class}\n",
                node.id,
                escape_label(&node.label)
            ));
        }
        for (from, to) in &self.edges {
            out.push_str(&format!("  {from} --> {to}\n"));
        }
        out
    }

    fn value(&self) -> Value {
        let nodes: Vec<Value> = self
            .nodes
            .iter()
            .map(|n| match &n.manifest {
                Some(manifest) => {
                    json!({"id": n.id, "label": n.label, "kind": "workspace", "manifest": manifest})
                }
                None => json!({"id": n.id, "label": n.label, "kind": "external"}),
            })
            .collect();
        let edges: Vec<Value> = self
            .edges
            .iter()
            .map(|(from, to)| json!({"from": from, "to": to}))
            .collect();
        json!({"nodes": nodes, "edges": edges})
    }
}

fn transition_label(t: &Transition) -> String {
    if t.commands.is_empty() {
        t.input.clone()
    } else {
        format!("{} / {}", t.input, t.commands.join(", "))
    }
}

fn state_diagram(machine: &Machine) -> String {
    let mut out: String = String::from("stateDiagram-v2\n");
    for state in &machine.states {
        out.push_str(&format!(
            "  state \"{}\" as {}\n",
            escape_label(&state.label),
            state.id
        ));
    }
    for t in &machine.transitions {
        out.push_str(&format!(
            "  {} --> {} : {}\n",
            t.from,
            t.to,
            escape_label(&transition_label(t))
        ));
    }
    out
}

fn machine_value(machine: &Machine) -> Value {
    let states: Vec<Value> = machine
        .states
        .iter()
        .map(|s| json!({"id": s.id, "label": s.label}))
        .collect();
    let transitions: Vec<Value> = machine
        .transitions
        .iter()
        .map(|t| json!({"from": t.from, "to": t.to, "input": t.input, "commands": t.commands}))
        .collect();
    json!({"states": states, "transitions": transitions})
}

#[cfg(test)]
mod tests {
    use super::{Files, render, stream_input, stream_phase};
    use crate::machine::{Machine, State, Transition};
    use crate::workspace::{Member, Package, Workspace};
    use phonepad_core::stream::{Offer, Sent, Stream, StreamInput, UnitKind};
    use std::collections::{BTreeMap, BTreeSet};

    fn package(name: &str, version: &str) -> Package {
        Package {
            name: name.to_string(),
            version: version.to_string(),
        }
    }

    fn workspace() -> Workspace {
        Workspace {
            members: vec![
                Member {
                    package: package("app", "0.1.0"),
                    manifest: String::from("crates/app/Cargo.toml"),
                    dependencies: BTreeSet::from([
                        package("core", "0.1.0"),
                        package("serde-json", "1.0.2"),
                    ]),
                },
                Member {
                    package: package("core", "0.1.0"),
                    manifest: String::from("crates/core/Cargo.toml"),
                    dependencies: BTreeSet::new(),
                },
            ],
        }
    }

    fn lamp() -> BTreeMap<String, Machine> {
        BTreeMap::from([(
            String::from("lamp"),
            Machine {
                states: vec![
                    State {
                        id: String::from("s_Off"),
                        label: String::from("Off"),
                    },
                    State {
                        id: String::from("s_On"),
                        label: String::from("On { at: 1 }"),
                    },
                ],
                transitions: vec![
                    Transition {
                        from: String::from("s_Off"),
                        to: String::from("s_On"),
                        input: String::from("Press { at: 1 }"),
                        commands: vec![String::from("Power(true)"), String::from("Log")],
                    },
                    Transition {
                        from: String::from("s_On"),
                        to: String::from("s_On"),
                        input: String::from("Tick"),
                        commands: vec![],
                    },
                ],
            },
        )])
    }

    #[test]
    fn crate_graph_styles_members_and_versions_externals() {
        let files: Files = render(&workspace(), &BTreeMap::new());
        let expected: &str = "flowchart LR\n  classDef workspace fill:#2b6cb0,stroke:#1a365d,color:#ffffff\n  classDef external fill:#edf2f7,stroke:#a0aec0,color:#1a202c\n  c_app[\"app\"]:::workspace\n  c_core[\"core\"]:::workspace\n  c_serde_json_1_0_2[\"serde-json 1.0.2\"]:::external\n  c_app --> c_core\n  c_app --> c_serde_json_1_0_2\n";
        assert_eq!(files["crate-graph.mmd"], expected);
    }

    #[test]
    fn state_diagram_declares_states_and_labels_transitions() {
        let files: Files = render(&workspace(), &lamp());
        let expected: &str = "stateDiagram-v2\n  state \"Off\" as s_Off\n  state \"On { at: 1 }\" as s_On\n  s_Off --> s_On : Press { at: 1 } / Power(true), Log\n  s_On --> s_On : Tick\n";
        assert_eq!(files["lamp.mmd"], expected);
    }

    #[test]
    fn views_json_carries_manifests_and_transitions() {
        let files: Files = render(&workspace(), &lamp());
        let views: serde_json::Value = serde_json::from_str(&files["views.json"]).unwrap();
        assert_eq!(
            (
                &views["crates"]["nodes"][0]["manifest"],
                &views["crates"]["nodes"][2]["kind"],
                &views["machines"]["lamp"]["transitions"][0]["commands"][1]
            ),
            (
                &serde_json::json!("crates/app/Cargo.toml"),
                &serde_json::json!("external"),
                &serde_json::json!("Log")
            )
        );
    }

    #[test]
    fn render_without_machines_emits_only_crate_files() {
        let files: Files = render(&workspace(), &BTreeMap::new());
        assert_eq!(
            files.keys().cloned().collect::<Vec<String>>(),
            vec![String::from("crate-graph.mmd"), String::from("views.json")]
        );
    }

    #[test]
    fn a_stream_state_is_drawn_as_its_phase() {
        assert_eq!(stream_phase(&Stream::Hidden), "Hidden");
    }

    #[test]
    fn stream_inputs_are_drawn_without_their_clock() {
        assert_eq!(
            [
                StreamInput::Visibility(true),
                StreamInput::UnitReady(Offer {
                    kind: UnitKind::Delta,
                    unit: Sent {
                        id: 3,
                        bytes: 1000,
                        at_us: 7
                    }
                }),
                StreamInput::Missed,
                StreamInput::Ack { id: 2, at_us: 9 }
            ]
            .map(|input: StreamInput| stream_input(&input)),
            [
                String::from("Visibility(true)"),
                String::from("UnitReady(Delta, 1000 B)"),
                String::from("Missed"),
                String::from("Ack(2)")
            ]
        );
    }
}
