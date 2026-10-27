use super::*;
use std::{collections::BTreeMap, sync::Arc};
use synergos_net::content::Block;

#[test]
fn compression_is_selective_deterministic_and_bounded() {
    let plain = vec![b'x'; 64 * 1024];
    let (kind, wire) = codec::choose(&plain).unwrap();
    assert_eq!(kind, Encoding::Gzip);
    assert_eq!(codec::encode(&plain, kind).unwrap(), wire);
    assert_eq!(codec::decode(&wire, kind, plain.len()).unwrap(), plain);
    assert!(codec::decode(&wire, kind, 32).is_err());
    assert!(codec::decode(&wire, kind, MAX_BLOCK_BYTES + 1).is_err());
    assert_eq!(codec::choose(b"tiny").unwrap().0, Encoding::Identity);
    let noise: Vec<u8> = (0..2048u64)
        .flat_map(|n| *blake3::hash(&n.to_le_bytes()).as_bytes())
        .collect();
    assert_eq!(codec::choose(&noise).unwrap().0, Encoding::Identity);
}

#[tokio::test]
async fn preparation_reuses_blocks_and_preserves_legacy_root() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source");
    tokio::fs::write(&input, vec![42; 3 * 65536]).await.unwrap();
    let cache = Arc::new(BlockCache::open(&dir.path().join("cache")).unwrap());
    assert!(BlockCache::open(&dir.path().join("cache")).is_err());
    let manifest = prepare(&cache, &input, 65536).await.unwrap();
    assert_eq!(manifest.blocks.len(), 2); // three identical chunks share one block
    let dag = assembly::validated_dag(&cache, &manifest).await.unwrap();
    assert_eq!(dag.chunks.len(), 3);
    let root = cache.load(&manifest.root).await.unwrap().unwrap();
    let legacy = synergos_net::content::MemoryContentStore::new();
    let legacy_root = synergos_net::content::add_file(
        &legacy,
        &input,
        synergos_net::content::ChunkerOptions { chunk_size: 65536 },
    )
    .await
    .unwrap();
    assert_eq!(root.cid, legacy_root);
    let destination = dir.path().join("output");
    let report = pull(
        cache.clone(),
        TransferPlan {
            manifest: manifest.clone(),
            grants: vec![],
        },
        &manifest.root,
        &destination,
        TransferOptions {
            allowed_origins: vec!["https://example.invalid".into()],
            ca_bundle: None,
            concurrency: 2,
            retries: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(report.wire_bytes, 0);
    assert_eq!(
        tokio::fs::read(&destination).await.unwrap(),
        tokio::fs::read(&input).await.unwrap()
    );
    assert!(assembly::assemble(&cache, &dag, &destination)
        .await
        .is_err());
    assert_eq!(
        tokio::fs::read(&destination).await.unwrap().len(),
        3 * 65536
    );
    assert!(prepare(&cache, &input, 0).await.is_err());
}

#[tokio::test]
async fn corrupted_cached_block_is_not_resume_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let cache = BlockCache::open(dir.path()).unwrap();
    let block = Block::new(b"checked".to_vec());
    cache.save(block.clone()).await.unwrap();
    tokio::fs::write(dir.path().join(&block.cid.0), b"corrupt")
        .await
        .unwrap();
    assert!(cache.load(&block.cid.0).await.unwrap().is_none());
    cache.save(block.clone()).await.unwrap();
    assert_eq!(
        cache.load(&block.cid.0).await.unwrap().unwrap().bytes,
        b"checked"
    );
    assert!(cache.load("../../outside").await.is_err());
}

#[test]
fn origin_policy_rejects_cleartext_redirect_targets_and_header_injection() {
    let client = http::HttpClient::new(&["https://storage.example".into()], None).unwrap();
    assert!(client
        .validate(
            "https://storage.example/object?signature=secret",
            &BTreeMap::new()
        )
        .is_ok());
    assert!(client
        .validate("https://other.example/object", &BTreeMap::new())
        .is_err());
    assert!(client
        .validate("http://storage.example/object", &BTreeMap::new())
        .is_err());
    assert!(client
        .validate(
            "https://user:secret@storage.example/object",
            &BTreeMap::new()
        )
        .is_err());
    assert!(client
        .validate(
            "https://storage.example/object",
            &BTreeMap::from([("Host".into(), "other.example".into())])
        )
        .is_err());
    let error = client
        .validate(
            "https://storage.example/object",
            &BTreeMap::from([("Authorization".into(), "secret\r\nInjected: yes".into())]),
        )
        .unwrap_err();
    assert!(!error.to_string().contains("secret"));
    let dir = tempfile::tempdir().unwrap();
    let invalid_ca = dir.path().join("invalid.pem");
    std::fs::write(&invalid_ca, b"not a certificate").unwrap();
    assert!(http::HttpClient::new(&["https://storage.example".into()], Some(&invalid_ca)).is_err());
}

#[tokio::test]
async fn empty_file_and_wrong_root_and_incomplete_closure() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("empty");
    tokio::fs::write(&source, []).await.unwrap();
    let cache = BlockCache::open(&dir.path().join("cache")).unwrap();
    let mut manifest = prepare(&cache, &source, 65536).await.unwrap();
    assert_eq!(manifest.blocks.len(), 1);
    let mut duplicate = manifest.clone();
    duplicate.blocks.push(duplicate.blocks[0].clone());
    assert!(duplicate.validate(&manifest.root).is_err());
    let mut oversized = manifest.clone();
    oversized.blocks[0].wire_bytes = MAX_BLOCK_BYTES + 1;
    assert!(oversized.validate(&manifest.root).is_err());
    assert!(manifest
        .validate(&Block::new(b"other".to_vec()).cid.0)
        .is_err());
    let dag = assembly::validated_dag(&cache, &manifest).await.unwrap();
    assembly::assemble(&cache, &dag, &dir.path().join("output"))
        .await
        .unwrap();
    manifest.file_bytes = 1;
    assert!(assembly::validated_dag(&cache, &manifest).await.is_err());
}
