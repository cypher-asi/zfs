use tempfile::TempDir;
use zfs_core::{ProgramId, SectorId};

use crate::{RocksStorage, SectorStore, StorageConfig};

fn open_temp_db() -> (RocksStorage, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let config = StorageConfig::new(dir.path());
    let storage = RocksStorage::open(config).expect("open");
    (storage, dir)
}

fn test_pid() -> ProgramId {
    ProgramId::from([0x11; 32])
}

fn test_sid() -> SectorId {
    SectorId::from_bytes(vec![0xAA; 32])
}

#[test]
fn append_and_read_log() {
    let (db, _dir) = open_temp_db();
    let pid = test_pid();
    let sid = test_sid();

    assert_eq!(db.log_length(&pid, &sid).expect("len"), 0);

    let idx0 = db.append(&pid, &sid, b"entry-0").expect("append");
    assert_eq!(idx0, 0);

    let idx1 = db.append(&pid, &sid, b"entry-1").expect("append");
    assert_eq!(idx1, 1);

    assert_eq!(db.log_length(&pid, &sid).expect("len"), 2);

    let entries = db.read_log(&pid, &sid, 0, 10).expect("read");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0], b"entry-0");
    assert_eq!(entries[1], b"entry-1");
}

#[test]
fn read_log_with_offset() {
    let (db, _dir) = open_temp_db();
    let pid = test_pid();
    let sid = test_sid();

    for i in 0..5u8 {
        db.append(&pid, &sid, &[i]).expect("append");
    }

    let entries = db.read_log(&pid, &sid, 2, 10).expect("read");
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0], &[2]);
    assert_eq!(entries[1], &[3]);
    assert_eq!(entries[2], &[4]);
}

#[test]
fn read_log_max_entries() {
    let (db, _dir) = open_temp_db();
    let pid = test_pid();
    let sid = test_sid();

    for i in 0..10u8 {
        db.append(&pid, &sid, &[i]).expect("append");
    }

    let entries = db.read_log(&pid, &sid, 0, 3).expect("read");
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0], &[0]);
    assert_eq!(entries[2], &[2]);
}

#[test]
fn insert_at_idempotent() {
    let (db, _dir) = open_temp_db();
    let pid = test_pid();
    let sid = test_sid();

    assert!(db.insert_at(&pid, &sid, 0, b"first").expect("insert"));
    assert!(!db.insert_at(&pid, &sid, 0, b"dup").expect("insert"));

    let entries = db.read_log(&pid, &sid, 0, 10).expect("read");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], b"first");
}

#[test]
fn sector_stats() {
    let (db, _dir) = open_temp_db();
    let pid = test_pid();
    let sid1 = SectorId::from_bytes(vec![0xAA; 32]);
    let sid2 = SectorId::from_bytes(vec![0xBB; 32]);

    db.append(&pid, &sid1, b"aaa").expect("append");
    db.append(&pid, &sid1, b"bbb").expect("append");
    db.append(&pid, &sid2, b"ccc").expect("append");

    let stats = db.sector_stats().expect("stats");
    assert_eq!(stats.sector_count, 2);
    assert_eq!(stats.entry_count, 3);
    assert_eq!(stats.sector_size_bytes, 9);
}

#[test]
fn list_programs_and_sectors() {
    let (db, _dir) = open_temp_db();
    let pid1 = ProgramId::from([0x11; 32]);
    let pid2 = ProgramId::from([0x22; 32]);
    let sid = test_sid();

    db.append(&pid1, &sid, b"a").expect("append");
    db.append(&pid2, &sid, b"b").expect("append");

    let programs = db.list_programs().expect("list");
    assert_eq!(programs.len(), 2);

    let sectors = db.list_sectors(&pid1).expect("list");
    assert_eq!(sectors.len(), 1);
    assert_eq!(sectors[0], sid);
}

#[test]
fn reopen_persists() {
    let dir = TempDir::new().expect("tempdir");
    let pid = test_pid();
    let sid = test_sid();

    {
        let db = RocksStorage::open(StorageConfig::new(dir.path())).expect("open");
        db.append(&pid, &sid, b"persistent").expect("append");
    }

    {
        let db = RocksStorage::open(StorageConfig::new(dir.path())).expect("reopen");
        let entries = db.read_log(&pid, &sid, 0, 10).expect("read");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], b"persistent");
    }
}

#[test]
fn rejects_non_32_byte_sector_id() {
    let (db, _dir) = open_temp_db();
    let pid = test_pid();
    let bad_sid = SectorId::from_bytes(b"too-short".to_vec());
    assert!(db.append(&pid, &bad_sid, b"data").is_err());
}

#[test]
fn multiple_sectors_per_program() {
    let (db, _dir) = open_temp_db();
    let pid = test_pid();
    let sid1 = SectorId::from_bytes(vec![0x01; 32]);
    let sid2 = SectorId::from_bytes(vec![0x02; 32]);

    db.append(&pid, &sid1, b"s1-e0").expect("append");
    db.append(&pid, &sid2, b"s2-e0").expect("append");
    db.append(&pid, &sid1, b"s1-e1").expect("append");

    assert_eq!(db.log_length(&pid, &sid1).expect("len"), 2);
    assert_eq!(db.log_length(&pid, &sid2).expect("len"), 1);

    let sectors = db.list_sectors(&pid).expect("list");
    assert_eq!(sectors.len(), 2);
}
