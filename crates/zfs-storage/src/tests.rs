use tempfile::TempDir;
use zfs_core::{ProgramId, SectorId};

use crate::{RocksStorage, SectorStore, StorageConfig};

fn open_temp_db() -> (RocksStorage, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let config = StorageConfig::new(dir.path());
    let storage = RocksStorage::open(config).expect("open");
    (storage, dir)
}

#[test]
fn sector_put_get() {
    let (db, _dir) = open_temp_db();
    let pid = ProgramId::from([0x11; 32]);
    let sid = SectorId::from_bytes(b"sector-1".to_vec());

    assert!(db.get(&pid, &sid).expect("get").is_none());

    db.put(&pid, &sid, b"payload-data", false, None)
        .expect("put");
    let got = db.get(&pid, &sid).expect("get").expect("some");
    assert_eq!(got, b"payload-data");
}

#[test]
fn sector_write_once_rejects_overwrite() {
    let (db, _dir) = open_temp_db();
    let pid = ProgramId::from([0x22; 32]);
    let sid = SectorId::from_bytes(b"once".to_vec());

    db.put(&pid, &sid, b"v1", false, None).expect("put");
    let err = db.put(&pid, &sid, b"v2", false, None);
    assert!(err.is_err());
}

#[test]
fn sector_overwrite_allowed() {
    let (db, _dir) = open_temp_db();
    let pid = ProgramId::from([0x33; 32]);
    let sid = SectorId::from_bytes(b"mutable".to_vec());

    db.put(&pid, &sid, b"v1", false, None).expect("put");
    db.put(&pid, &sid, b"v2", true, None).expect("overwrite");
    let got = db.get(&pid, &sid).expect("get").expect("some");
    assert_eq!(got, b"v2");
}

#[test]
fn sector_stats() {
    let (db, _dir) = open_temp_db();
    let pid = ProgramId::from([0x44; 32]);

    db.put(
        &pid,
        &SectorId::from_bytes(b"s1".to_vec()),
        b"aaa",
        false,
        None,
    )
    .expect("put");
    db.put(
        &pid,
        &SectorId::from_bytes(b"s2".to_vec()),
        b"bbb",
        false,
        None,
    )
    .expect("put");

    let stats = db.sector_stats().expect("stats");
    assert_eq!(stats.sector_count, 2);
    assert_eq!(stats.sector_size_bytes, 6);
}

#[test]
fn sector_batch_put_and_get() {
    let (db, _dir) = open_temp_db();
    let pid = ProgramId::from([0x55; 32]);
    let entries = vec![
        (
            SectorId::from_bytes(b"b1".to_vec()),
            b"data1".to_vec(),
            false,
            None,
        ),
        (
            SectorId::from_bytes(b"b2".to_vec()),
            b"data2".to_vec(),
            false,
            None,
        ),
    ];

    let results = db.batch_put(&pid, &entries).expect("batch_put");
    assert!(results.iter().all(|r| r.ok));

    let sids = vec![
        SectorId::from_bytes(b"b1".to_vec()),
        SectorId::from_bytes(b"b2".to_vec()),
    ];
    let payloads = db.batch_get(&pid, &sids).expect("batch_get");
    assert_eq!(payloads[0].as_deref(), Some(b"data1".as_slice()));
    assert_eq!(payloads[1].as_deref(), Some(b"data2".as_slice()));
}

#[test]
fn reopen_persists_sector_data() {
    let dir = TempDir::new().expect("tempdir");
    let pid = ProgramId::from([0x66; 32]);
    let sid = SectorId::from_bytes(b"persist".to_vec());

    {
        let config = StorageConfig::new(dir.path());
        let db = RocksStorage::open(config).expect("open");
        db.put(&pid, &sid, b"persistent", false, None)
            .expect("put");
    }

    {
        let config = StorageConfig::new(dir.path());
        let db = RocksStorage::open(config).expect("reopen");
        let got = db.get(&pid, &sid).expect("get").expect("some");
        assert_eq!(got, b"persistent");
    }
}
