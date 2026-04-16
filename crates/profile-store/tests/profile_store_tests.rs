use domain::AuthFile;
use profile_store::ProfileStore;
use std::fs;

#[test]
fn saves_and_lists_profiles() {
    let temp = tempfile::tempdir().unwrap();
    let auth_dir = temp.path().join(".codex");
    let data_dir = temp.path().join("shapeshifter");
    let store = ProfileStore::from_dirs(auth_dir, data_dir.clone());
    fs::create_dir_all(data_dir.join("accounts")).unwrap();

    let auth = AuthFile::default();
    store.save_profile("alice", &auth).unwrap();
    store.save_profile("bob", &auth).unwrap();

    let profiles = store.list_profiles().unwrap();
    let labels = profiles
        .into_iter()
        .map(|profile| profile.label)
        .collect::<Vec<_>>();
    assert_eq!(labels, vec!["alice".to_string(), "bob".to_string()]);
}

#[test]
fn loads_hosts_with_local_first() {
    let temp = tempfile::tempdir().unwrap();
    let auth_dir = temp.path().join(".codex");
    let data_dir = temp.path().join("shapeshifter");
    let store = ProfileStore::from_dirs(auth_dir.clone(), data_dir.clone());
    fs::create_dir_all(data_dir.join("accounts")).unwrap();
    fs::write(data_dir.join("accounts/.hosts"), "vd\nneo\n").unwrap();

    let hosts = store.load_hosts().unwrap();
    assert_eq!(hosts[0].label, "local");
    assert_eq!(hosts[1].label, "vd");
    assert_eq!(hosts[2].label, "neo");
    match &hosts[0].target {
        domain::HostTarget::Local { auth_file_path } => {
            assert_eq!(auth_file_path, &auth_dir.join("auth.json"));
        }
        domain::HostTarget::Remote(_) => panic!("expected local host"),
    }
}

#[cfg(windows)]
#[test]
fn loads_remote_hosts_with_custom_auth_paths() {
    let temp = tempfile::tempdir().unwrap();
    let auth_dir = temp.path().join(".codex");
    let data_dir = temp.path().join("shapeshifter");
    let store = ProfileStore::from_dirs(auth_dir, data_dir.clone());
    fs::create_dir_all(data_dir.join("accounts")).unwrap();

    let user_profile = std::env::var("USERPROFILE").unwrap_or_else(|_| {
        dirs::home_dir()
            .expect("expected current Windows user home directory")
            .display()
            .to_string()
    });
    let auth_path = format!("{user_profile}\\.local\\share\\opencode\\auth.json");
    let managed_dir = format!("{user_profile}\\AppData\\Local\\shapeshifter");
    fs::write(
        data_dir.join("accounts/.hosts"),
        format!("vd\nwinbox {auth_path} {managed_dir}\n"),
    )
    .unwrap();

    let hosts = store.load_hosts().unwrap();
    assert_eq!(hosts[1].label, "vd");

    let remote = match &hosts[2].target {
        domain::HostTarget::Remote(remote) => remote,
        domain::HostTarget::Local { .. } => panic!("expected remote host"),
    };
    assert_eq!(remote.ssh_alias, "winbox");
    assert_eq!(remote.auth_file_path, std::path::PathBuf::from(auth_path));
    assert_eq!(
        remote.managed_data_dir,
        std::path::PathBuf::from(managed_dir)
    );
}

#[test]
fn deletes_profile_file() {
    let temp = tempfile::tempdir().unwrap();
    let auth_dir = temp.path().join(".codex");
    let data_dir = temp.path().join("shapeshifter");
    let store = ProfileStore::from_dirs(auth_dir, data_dir.clone());
    fs::create_dir_all(data_dir.join("accounts")).unwrap();

    let auth = AuthFile::default();
    store.save_profile("alice", &auth).unwrap();
    store.delete_profile("alice").unwrap();

    let profiles = store.list_profiles().unwrap();
    assert!(profiles.is_empty());
}
