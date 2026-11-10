use std::os::unix::fs::{PermissionsExt, symlink};

use super::*;

struct Rig {
    dir: PathBuf,
    marker: PathBuf,
}

impl Rig {
    fn new(name: &str) -> Rig {
        let dir: PathBuf =
            std::env::temp_dir().join(format!("phonepad-owe-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        symlink(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/owe/stub.sh"),
            dir.join("owe"),
        )
        .unwrap();
        Rig {
            marker: dir.join(MARKER_DIR).join(MARKER_FILE),
            dir,
        }
    }

    fn shell(&self, state: Wallpaper) -> WallpaperShell {
        WallpaperShell {
            owe: self.dir.join("owe").to_string_lossy().into_owned(),
            state,
            marker: Some(self.marker.clone()),
            held: None,
            unavailable_logged: false,
        }
    }

    fn paused(&self) -> bool {
        self.dir.join("paused").exists()
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.dir.join("calls")).unwrap_or_default()
    }

    fn set_paused(&self) {
        std::fs::write(self.dir.join("paused"), b"").unwrap();
    }
}

#[test]
fn manual_pause_off() {
    assert_eq!(
        manual_pause(br#"{"status":"ok","manual_pause":false}"#),
        Ok(false)
    );
}

#[test]
fn manual_pause_on() {
    assert_eq!(
        manual_pause(br#"{"status":"ok","manual_pause":true}"#),
        Ok(true)
    );
}

#[test]
fn missing_manual_pause_is_an_error() {
    assert!(manual_pause(br#"{"status":"ok","paused":true}"#).is_err());
}

#[test]
fn empty_status_is_an_error() {
    assert!(manual_pause(b"").is_err());
}

#[test]
fn no_marker_file_is_absent() {
    assert_eq!(probe(&Rig::new("absent").marker), Ok(Marker::Absent));
}

#[test]
fn a_marker_held_by_a_live_owner_is_live() {
    let rig: Rig = Rig::new("live");
    let held: File = take(&rig.marker).unwrap();
    assert_eq!(probe(&rig.marker), Ok(Marker::Live));
    drop(held);
}

#[test]
fn a_marker_whose_owner_died_is_stale() {
    let rig: Rig = Rig::new("stale");
    take(&rig.marker).unwrap().unlock().unwrap();
    assert_eq!(probe(&rig.marker), Ok(Marker::Stale));
}

#[test]
fn an_empty_unlocked_marker_is_absent() {
    let rig: Rig = Rig::new("empty");
    std::fs::create_dir_all(rig.marker.parent().unwrap()).unwrap();
    std::fs::write(&rig.marker, b"").unwrap();
    assert_eq!(probe(&rig.marker), Ok(Marker::Absent));
}

#[test]
fn a_second_owner_cannot_take_a_live_marker() {
    let rig: Rig = Rig::new("second");
    let held: File = take(&rig.marker).unwrap();
    assert!(take(&rig.marker).is_err());
    drop(held);
}

#[test]
fn a_stale_marker_can_be_taken_over() {
    let rig: Rig = Rig::new("takeover");
    take(&rig.marker).unwrap().unlock().unwrap();
    let held: File = take(&rig.marker).unwrap();
    assert_eq!(probe(&rig.marker), Ok(Marker::Live));
    drop(held);
}

#[test]
fn the_marker_names_its_owner_once() {
    let rig: Rig = Rig::new("pid");
    take(&rig.marker).unwrap().unlock().unwrap();
    let held: File = take(&rig.marker).unwrap();
    assert_eq!(
        std::fs::read_to_string(&rig.marker).unwrap(),
        format!("{}\n", std::process::id())
    );
    drop(held);
}

#[test]
fn clearing_a_live_peers_marker_leaves_it() {
    let rig: Rig = Rig::new("clear-live");
    let peer: File = take(&rig.marker).unwrap();
    let mut shell: WallpaperShell = rig.shell(Wallpaper::Held);
    assert_eq!(
        (shell.clear_marker().is_err(), probe(&rig.marker)),
        (true, Ok(Marker::Live))
    );
    drop(peer);
}

#[tokio::test]
async fn startup_over_a_crashed_owners_marker_resumes_and_clears_it() {
    let rig: Rig = Rig::new("startup-stale");
    rig.set_paused();
    take(&rig.marker).unwrap().unlock().unwrap();
    let mut shell: WallpaperShell = rig.shell(Wallpaper::Idle);
    shell.startup().await;
    assert_eq!(
        (rig.paused(), probe(&rig.marker), rig.calls()),
        (false, Ok(Marker::Absent), String::from("resume\n"))
    );
}

#[tokio::test]
async fn startup_beside_a_live_owner_leaves_its_pause() {
    let rig: Rig = Rig::new("startup-live");
    rig.set_paused();
    let peer: File = take(&rig.marker).unwrap();
    let mut shell: WallpaperShell = rig.shell(Wallpaper::Idle);
    shell.startup().await;
    assert_eq!((rig.paused(), rig.calls()), (true, String::new()));
    drop(peer);
}

#[tokio::test]
async fn capture_start_pauses_and_holds_the_marker() {
    let rig: Rig = Rig::new("start");
    let mut shell: WallpaperShell = rig.shell(Wallpaper::Idle);
    shell.capture_started().await;
    assert_eq!(
        (rig.paused(), probe(&rig.marker), shell.state),
        (true, Ok(Marker::Live), Wallpaper::Held)
    );
}

#[tokio::test]
async fn a_tick_reasserts_a_pause_someone_else_lifted() {
    let rig: Rig = Rig::new("tick");
    let mut shell: WallpaperShell = rig.shell(Wallpaper::Idle);
    shell.capture_started().await;
    std::fs::remove_file(rig.dir.join("paused")).unwrap();
    shell.tick().await;
    assert_eq!(rig.calls(), String::from("status\npause\nstatus\npause\n"));
}

#[tokio::test]
async fn capture_stop_resumes_and_releases_the_marker() {
    let rig: Rig = Rig::new("stop");
    let mut shell: WallpaperShell = rig.shell(Wallpaper::Idle);
    shell.capture_started().await;
    shell.capture_stopped().await;
    assert_eq!(
        (rig.paused(), probe(&rig.marker), shell.state),
        (false, Ok(Marker::Absent), Wallpaper::Idle)
    );
}

#[tokio::test]
async fn an_unreachable_owe_changes_nothing() {
    let rig: Rig = Rig::new("unreachable");
    let mut shell: WallpaperShell = rig.shell(Wallpaper::Idle);
    shell.owe = rig.dir.join("missing").to_string_lossy().into_owned();
    shell.capture_started().await;
    assert_eq!(
        (shell.state, probe(&rig.marker), shell.unavailable_logged),
        (Wallpaper::Idle, Ok(Marker::Absent), true)
    );
}

#[tokio::test]
async fn a_tick_after_owe_came_back_starts_the_pause_it_missed() {
    let rig: Rig = Rig::new("came-back");
    let mut shell: WallpaperShell = rig.shell(Wallpaper::Idle);
    shell.owe = rig.dir.join("missing").to_string_lossy().into_owned();
    shell.capture_started().await;
    shell.owe = rig.dir.join("owe").to_string_lossy().into_owned();
    shell.tick().await;
    assert_eq!((shell.state, rig.paused()), (Wallpaper::Held, true));
}

#[tokio::test]
async fn no_runtime_dir_leaves_the_wallpaper_alone() {
    let rig: Rig = Rig::new("no-runtime");
    let mut shell: WallpaperShell = rig.shell(Wallpaper::Idle);
    shell.marker = None;
    shell.startup().await;
    shell.capture_started().await;
    assert_eq!(
        (shell.state, rig.calls(), shell.unavailable_logged),
        (Wallpaper::Idle, String::new(), true)
    );
}

#[test]
fn unavailability_is_logged_once() {
    let rig: Rig = Rig::new("logged");
    let mut shell: WallpaperShell = rig.shell(Wallpaper::Idle);
    shell.unavailable("first");
    shell.unavailable("second");
    assert!(shell.unavailable_logged);
}

#[test]
fn an_unreadable_marker_is_an_error_not_absent() {
    let rig: Rig = Rig::new("unreadable");
    std::fs::create_dir_all(rig.marker.parent().unwrap()).unwrap();
    std::fs::write(&rig.marker, b"1\n").unwrap();
    std::fs::set_permissions(&rig.marker, std::fs::Permissions::from_mode(0o000)).unwrap();
    assert!(probe(&rig.marker).is_err());
}
