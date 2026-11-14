#![cfg(unix)]
use celetixo_events::worker;
use std::os::unix::fs::{PermissionsExt, symlink};

fn configuration() -> serde_json::Value {
    serde_json::json!({"contract_version":1,"organization_id":"org","workspace_id":"workspace","repository_id":"repository",
        "service_id":"publisher","service_kind":"Publisher","poll_interval_ms":100,"reconcile_interval_ms":1000,
        "database":{"directory":"/private/socket","database":"database","user":"publisher","password_file":"/private/password"},
        "broker":{"servers":["tls://localhost:4222"],"ca_file":"/private/ca","certificate":"/private/cert","private_key":"/private/key",
            "subject":"scope.lifecycle","stream":"LIFECYCLE","consumer":"PROJECTION","inbox_prefix":"_INBOX.publisher"}})
}

#[test]
fn operator_configuration_rejects_public_symlink_hardlink_oversized_and_unknown_inputs() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("service.json");
    let store = |value: &serde_json::Value| {
        std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    };
    store(&configuration());
    assert!(worker::load(&path).is_ok());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(worker::load(&path).is_err());
    store(&configuration());
    let alias = root.path().join("alias");
    symlink(&path, &alias).unwrap();
    assert!(worker::load(&alias).is_err());
    let hardlink = root.path().join("hardlink");
    std::fs::hard_link(&path, &hardlink).unwrap();
    assert!(worker::load(&path).is_err());
    std::fs::remove_file(hardlink).unwrap();
    let mut invalid = configuration();
    invalid["production_password"] = serde_json::json!("unrecognized");
    store(&invalid);
    assert!(worker::load(&path).is_err());
    for (field, value) in [
        ("contract_version", 2),
        ("poll_interval_ms", 0),
        ("reconcile_interval_ms", 0),
    ] {
        let mut invalid = configuration();
        invalid[field] = serde_json::json!(value);
        store(&invalid);
        assert!(worker::load(&path).is_err());
    }
    std::fs::write(&path, vec![b'x'; 65537]).unwrap();
    assert!(worker::load(&path).is_err());
}
