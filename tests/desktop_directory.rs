#![cfg(all(feature = "desktop", target_os = "linux", target_env = "gnu"))]
use mirelay::{
    bridge_registry::{BridgeRegistration, BridgeRegistryStore},
    config::{Config, InitOverrides, directory_state_dir},
    directory::{client::DirectoryClient, receiver::Receiver, sender::Sender},
    pairing::PairingClient,
    server::{ApiState, ServerStore, router},
};
use std::{fs, process::Command, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a graphical session; real tus → isolated GTK auto receiver"]
async fn gtk_auto_receives_directory_versions_and_keeps_conflict_copies_across_restart() {
    let root = tempfile::tempdir().unwrap();
    let store = ServerStore::new(root.path().join("server"), 104857600).unwrap();
    store.initialize().unwrap();
    let app = router(
        ApiState::new(store, "legacy".into(), "legacy-test-token")
            .unwrap()
            .with_admin_token("desktop-directory-graphical-administrator-test-only")
            .unwrap(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let result = tokio::task::spawn_blocking(move || {
        let sender_token = "2222222222222222222222222222222222222222222222222222222222222222";
        let admin = PairingClient::new(&base,"desktop-directory-graphical-administrator-test-only",true).unwrap(); let invitation = admin.create_folder("GTK directory QA").unwrap();
        let url = format!("{base}/f/{}",invitation.folder_id);
        let claimed = admin.claim(&invitation.pairing_code,sender_token).unwrap(); PairingClient::new(&url,&invitation.receiver_token,true).unwrap().confirm(claimed.verification.as_deref().unwrap()).unwrap();
        let source = root.path().join("source"); let destination = root.path().join("destination");
        fs::create_dir_all(source.join("Pixiv")).unwrap(); fs::create_dir_all(destination.join("Pixiv")).unwrap();
        fs::write(source.join("Pixiv/art.bin"),[0,255,0,127]).unwrap(); fs::write(destination.join("Pixiv/art.bin"),b"Linux original").unwrap();
        let mut config = Config::defaults(InitOverrides { data_dir:Some(root.path().join("desktop-state")),library_dir:Some(destination.clone()),..Default::default() }).unwrap();
        config.directory_sync=true; config.server=serde_json::from_value(serde_json::json!({"kind":"http","base_url":url,"token_env":"MIRELAY_GTK_TEST_TOKEN","allow_insecure_http":true})).unwrap();
        config.ensure_directories().unwrap(); let config_path=root.path().join("config.toml"); config.save(&config_path,false).unwrap();
        let transport=mirelay::http_source::HttpSource::new(&url,&invitation.receiver_token,5,100,true).unwrap(); let client=DirectoryClient::new(&url,&invitation.receiver_token,true,"receiver").unwrap();
        let mut receiver=Receiver::open(&destination,&directory_state_dir(&config),&url).unwrap(); receiver.sync(&client,&transport).unwrap(); drop(receiver);
        let registry=BridgeRegistryStore::new(root.path().join("folders.toml")); registry.update(|registry|registry.add(BridgeRegistration{kind:Default::default(),id:"directory".into(),name:"GTK directory QA".into(),config_path,auto_receive:true})).unwrap();
        let sender_client=DirectoryClient::new(&url,sender_token,true,"sender").unwrap(); let mut sender=Sender::open(&source,&root.path().join("sender-state"),&url).unwrap(); sender.preview(&sender_client).unwrap(); sender.initialize(&sender_client).unwrap();
        let launch=|| {
            let mut command=Command::new(env!("CARGO_BIN_EXE_mirelay-desktop")); command.args(["--new-instance","--registry"]).arg(registry.path()).arg("--automation-smoke-test").env("MIRELAY_GTK_TEST_TOKEN",&invitation.receiver_token);
            let output=assert_cmd::Command::from_std(command).timeout(Duration::from_secs(20)).output().unwrap();
            assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
        };
        assert_eq!(sender.send(&sender_client,sender_token,true).unwrap(),1); launch();
        let first=Receiver::inspect(&destination,&directory_state_dir(&config),&url).unwrap(); let entry=&first["Pixiv/art.bin"];
        assert!(entry.acknowledged && entry.conflict); assert_eq!(fs::read(destination.join(entry.history.as_ref().unwrap())).unwrap(),b"Linux original");
        fs::write(source.join("Pixiv/art.bin"),[1,254,1,126]).unwrap(); assert_eq!(sender.send(&sender_client,sender_token,true).unwrap(),1); launch();
        let second=Receiver::inspect(&destination,&directory_state_dir(&config),&url).unwrap(); let next=&second["Pixiv/art.bin"];
        assert!(next.acknowledged && !next.conflict && next.version>entry.version);
        assert_eq!(fs::read(destination.join("Pixiv/art.bin")).unwrap(),[1,254,1,126]);
        assert_eq!(fs::read(destination.join(next.history.as_ref().unwrap())).unwrap(),[0,255,0,127]);
        assert!(sender_client.state().unwrap().entries.iter().all(|entry|entry.acknowledged));
        assert!(!config.storage.state_file.exists());
    }).await;
    server.abort();
    result.unwrap();
}
