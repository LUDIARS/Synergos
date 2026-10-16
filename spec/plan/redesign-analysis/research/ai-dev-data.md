# AI-development needs for a version-control / asset platform (datasets, models, lineage, licensing, GPU access patterns)

Research notes for the Synergos (Rust, P2P/QUIC, BLAKE3 whole-file CAS history nodes, custody receipts) + Curare (Hono/PostgreSQL/MinIO catalog, working/submission/reproducible versioning, submission/review/release, Cernere identity) redesign. Date: 2026-10-15.

**Method note.** The shared WebSearch budget for this workflow turn was exhausted before this task started, so every source below was fetched directly by URL (primary vendor docs, specs, repos). Only `github.com`, `raw.githubusercontent.com` and `microsoft.com` resolved from this sandbox; vendor sites (huggingface.co, dvc.org, docs.lakefs.io, eur-lex, e-Gov Japan, Steam, Perforce, Adobe, Bynder, Thorn, arXiv, Wikipedia) did not resolve. Where a canonical page could not be fetched, the claim is marked **[NOT FETCHED]** and should be verified before it is used in a design document. Everything else was read from the URL given. Figures marked **(derived)** are my arithmetic from fetched numbers.

---

## 0. Key takeaways

1. The industry's "git for data" systems all converge on the same shape: **a small immutable manifest (pointer list with content hashes) stored in a code VCS or a metadata DB, plus a content-addressed blob store**. DVC `.dir` objects, Git LFS pointers, lakeFS commits, Iceberg manifest lists, Lance manifests and Croissant `FileObject.sha256` are all this pattern ([DVC internals](https://github.com/iterative/dvc.org/blob/main/content/docs/user-guide/project-structure/internal-files.md), [LFS spec](https://github.com/git-lfs/git-lfs/blob/main/docs/spec.md), [Iceberg spec](https://raw.githubusercontent.com/apache/iceberg/main/format/spec.md), [Lance table.proto](https://raw.githubusercontent.com/lancedb/lance/main/protos/table.proto), [Croissant](https://github.com/mlcommons/croissant)). Synergos's "git holds text + manifest of pointers, Synergos moves blobs" is the mainstream design; what differs is the *granularity* of the hash unit and the *retention/ref* semantics.
2. Hugging Face's Xet is the only hub-scale system that publishes full numbers for chunk-level CAS: 64 KiB GearHash chunks (8–128 KiB), BLAKE3-keyed 256-bit Merkle hashes, xorbs ≤ 64 MiB, shards ≤ 64 MiB, HMAC-protected global dedup, 30–85 % savings on fine-tuned checkpoints ([dedup spec](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/deduplication.md), [hashing](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/hashing.md), [blog](https://raw.githubusercontent.com/huggingface/blog/main/from-files-to-chunks.md)). Because Synergos has decided on whole-file transfer (2026-08-16), these numbers quantify what that decision costs *specifically for model checkpoints and append-only datasets*; for game binaries (textures, meshes) the data point is absent and should be measured on real studio data before revisiting.
3. "Millions of tiny files are the enemy" is quantifiable: S3 first-byte latency is 100–200 ms and a prefix sustains ≥ 5,500 GET/s ([S3 perf](https://github.com/awsdocs/amazon-s3-userguide/blob/main/doc_source/optimizing-performance.md)); the Hub caps a folder at 10k entries and recommends < 100k files per repo ([HF limits](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/hub/storage-limits.md)); Xet's stated design rule is "avoid any strategy that scales 1:1 with chunk count" ([blog](https://raw.githubusercontent.com/huggingface/blog/main/from-chunks-to-blocks.md)). Training loaders therefore want **shards** (WebDataset tar, MDS, parquet row groups of 512 MB–1 GB) and **range reads of 8–16 MB**, not per-sample objects.
4. Lineage is a solved vocabulary: OpenLineage `RunEvent{job, run, inputs[], outputs[]}` with a required-string `datasetVersion` facet and `dataQualityAssertions` facet ([object model](https://github.com/OpenLineage/OpenLineage/blob/main/spec/OpenLineage.md), [facet](https://raw.githubusercontent.com/OpenLineage/OpenLineage/main/spec/facets/DatasetVersionDatasetFacet.json)); MLflow `Dataset{name, digest, source}` + registry **aliases** (stages deprecated in 2.9.0) ([MLflow registry workflow](https://raw.githubusercontent.com/mlflow/mlflow/master/docs/docs/classic-ml/model-registry/workflow.mdx)). Curare should *emit* these, not reinvent them.
5. Governance is now a per-asset **rights/consent bitmap plus provenance record**: CAWG `training-mining` entries (`ai_inference`, `ai_generative_training`, `ai_training`, `data_mining`) each `allowed | notAllowed | constrained` ([CAWG spec](https://raw.githubusercontent.com/decentralized-identity/cawg-training-and-data-mining-assertion/main/docs/modules/ROOT/partials/assertion-definition.adoc)); C2PA `digitalSourceType = trainedAlgorithmicMedia` for generated outputs ([c2pa-rs actions](https://github.com/contentauth/c2pa-rs)); EU AI Act Art. 10 data-governance items (a)–(h) and Art. 53 training-content summary apply from 2 Aug 2026 / 2 Aug 2025 ([Art. 113 mirror](https://raw.githubusercontent.com/mcekikj/eu-ai-act-knowledge-graph/main/EU-AI-Act-Vault/01%20Articles/Article%20113.md)). **C2PA's reference SDK supports no 3D formats** ([supported formats](https://raw.githubusercontent.com/contentauth/c2pa-rs/main/docs/supported-formats.md)), so a game-asset platform must carry provenance as sidecar metadata in its own catalog.

---

## 1. How ML teams version data and models today

### 1.1 Pointer-in-git + content-addressed cache (Git LFS, DVC)

**Git LFS** pointer file: UTF-8 text < 1024 bytes, lines `version https://git-lfs.github.com/spec/v1`, `oid sha256:<hex>`, `size <bytes>`; local objects at `.git/lfs/objects/OID[0:2]/OID[2:4]/OID`; empty files bypass LFS ([spec](https://github.com/git-lfs/git-lfs/blob/main/docs/spec.md)). Whole-file dedup only.

**DVC** hashes are "a 32 characters long string (usually MD5)"; cache layout `.dvc/cache/files/md5/ec/1d29…`; a tracked directory gets a `.dir` entry that is a JSON array of `{md5, relpath}` — this is the manifest object ([internal files](https://github.com/iterative/dvc.org/blob/main/content/docs/user-guide/project-structure/internal-files.md)). `.dvc` file fields: `outs[].path` (required), `hash` (only `md5` supported), `md5|etag|checksum`, `size`, `nfiles`, `cache` (default true), `push` (default true), `remote`, `version_id`, `isexec`, `persist` ([.dvc files](https://github.com/iterative/dvc.org/blob/main/content/docs/user-guide/project-structure/dvc-files.md)). Pipelines: `dvc.yaml` stages (`cmd`, `deps`, `params`, `outs`, `frozen`, `always_changed`), `dvc.lock` records per-stage content hashes of deps/outs and param key+values; a change invalidates the stage ([dvc.yaml](https://raw.githubusercontent.com/iterative/dvc.org/main/content/docs/user-guide/project-structure/dvcyaml-files.md)). The **run cache** is keyed by a hash of exact dependency contents/param values and the command; entries are backups of `dvc.lock`, outputs live in the normal cache; `dvc push/pull --run-cache` transfers it ([internal files](https://github.com/iterative/dvc.org/blob/main/content/docs/user-guide/project-structure/internal-files.md)).

*Lesson for Synergos/Curare:* DVC proves the "directory manifest = JSON list of (path, hash)" model works at ML scale, but MD5, client-only enforcement and no server-side hooks are its weaknesses — exactly the gap a custody/control-plane service fills. The run-cache design (outputs addressed by hash of inputs+params+command) is the right primitive for Curare's `reproducible` versioning type.

### 1.2 Chunk-level CAS at hub scale: Hugging Face Xet (numbers to design against)

| Parameter | Value | Source |
|---|---|---|
| Chunking | content-defined, GearHash rolling hash; target 64 KiB, min 8 KiB, max 128 KiB (min not applied to last chunk / files < 8 KiB) | [dedup](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/deduplication.md), [upload](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/upload-protocol.md), [constants.rs `TARGET_CHUNK_SIZE = 64*1024`](https://github.com/huggingface/xet-core) |
| Chunk hash | BLAKE3 keyed mode with fixed 32-byte `DATA_KEY`; all hashes 256-bit | [hashing](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/hashing.md) |
| Xorb (CAS block) | ≤ 64 MiB serialized, ~1024 chunks (dedup doc: max 8,192 chunks); hash = Merkle root over `hex(child) : len\n` lines, BLAKE3-keyed with `INTERNAL_NODE_KEY`; chunk header 8 bytes (version, 3-byte compressed size, type, 3-byte uncompressed size); compression None / LZ4 / ByteGrouping4LZ4 chosen per chunk | [xorb](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/xorb.md), [hashing](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/hashing.md) |
| File hash | Merkle over chunk hashes, root re-hashed with all-zero key; optional SHA-256 required for git-backed repos (LFS pointer compatibility) | [hashing](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/hashing.md), [upload](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/upload-protocol.md) |
| Shard | ≤ 64 MiB; maps files → terms `(xorb_hash, [chunk_start, chunk_end))`; carries CAS-info about many xorbs | [dedup](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/deduplication.md) |
| Dedup levels | (1) current session in-memory, (2) local shard cache, (3) global API across all users/repos | same |
| Global dedup key chunks | first chunk of every file; else eligible if last 8 bytes of hash as LE u64 `% 1024 == 0`; ~1 query per 4 MB; async | same |
| Privacy of global dedup | chunk hashes HMAC'd with a per-shard key; raw hashes never sent server→client; a match reveals only the xorb + position | same |
| Fragmentation guard | prefer contiguous runs (≥ 8 chunks or ≥ 1 MB avg) over dedup hits | same |
| Endpoints | `GET /v1/chunks/default-merkledb/{chunk_hash}` (200 shard / 404); `POST /v1/xorbs/default/{xorb_hash}`; `POST /v1/shards` (registers files); `POST /v2/shards` streaming NDJSON progress; all idempotent on content hash; a shard referencing a missing xorb is rejected | [upload](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/upload-protocol.md) |
| Download | `GET /v2/reconstructions/{file_id}` (+ `Range: bytes=`) → ordered `terms[{hash, range[start,end), unpacked_length}]` + `xorbs{hash → [{url (presigned, X-Xet-Signed-Range, Expires, Signature), ranges[{chunks, bytes}]}]}`; multi-range GET returns `206 multipart/byteranges`; `offset_into_first_range` for partial reads; URLs short-lived, must not be cached; verify `Σ chunk lengths == unpacked_length` | [download](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/download-protocol.md) |
| Concurrency | adaptive: start low, scale with bandwidth, back off on congestion | [upload](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/upload-protocol.md) |

Measured effects: GPT-2 two `model.safetensors` versions 1.2 GB (LFS) → 645 MB (Xet), −53 %; fine-tunes/checkpoints 30–85 % dedup; PyTorch checkpoints ≈ 200 TB on the Hub; typical sizes parquet/CSV 200–300 MB, safetensors ≈ 1 GB, GGUF > 8 GB; CORD-19 50-update benchmark: download 51→19 min, upload 47→24 min, storage 8.9→3.52 GB ([from-files-to-chunks](https://raw.githubusercontent.com/huggingface/blog/main/from-files-to-chunks.md)). Gemma-2-9b GGUF repo, 29 quantizations: 191 GB → 97 GB (−49 %), upload at 50 MB/s 509→258 min; a 200 GB repo ≈ 3 M unique chunks; 45 PB across ~2 M repos would be ~690 B chunks — hence 64 MB blocks (≈1000× fewer CAS entries) ([from-chunks-to-blocks](https://raw.githubusercontent.com/huggingface/blog/main/from-chunks-to-blocks.md)). Operational lesson: unaligned 10 MB range requests against blocks lacking chunk-length metadata made CAS read 3.5–4× the bytes delivered until lengths were stored; fixing it cut GET latency ~35 % ([xet-on-the-hub](https://raw.githubusercontent.com/huggingface/blog/main/xet-on-the-hub.md)).

*Design implication:* Synergos already uses BLAKE3 content addressing; Xet shows that a **64 KiB chunk / 64 MiB block / ≤ 64 MiB manifest-shard** hierarchy keeps object counts tractable while still deduplicating checkpoints. If whole-file transfer is retained, at minimum store **per-file chunk-length tables** so that ranged reads can be served without over-reading, and keep the file hash a Merkle root so that partial verification is possible later without rehashing.

### 1.3 Hub limits, gating, cards (Hugging Face)

- Limits: 500 GB hard max per file (recommend < 200 GB); < 100k files per repo; **max 10k entries per folder**; < 100 files per commit; performance degrades after a few thousand commits; 60 s HTTP timeout; quotas e.g. Enterprise 200 TB public base + 1 TB/seat, private $18/TB/month ([storage-limits](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/hub/storage-limits.md)).
- Gated repos: access granted per *user* (not org); automatic or manual approval; API `/api/models/{repo_id}/user-access-request/{pending|accepted|rejected}` plus `handle` and `grant`; `reset` revokes; custom `extra_gated_fields` (`text|checkbox|date_picker|country|select`); `extra_gated_eu_disallowed: true` geo-blocks by IP; org-level gating and "Gating Group Collections" ([models-gated](https://github.com/huggingface/hub-docs/blob/main/docs/hub/models-gated.md)).
- Model card YAML: `license` (+`license_name`/`license_link` for `other`), `base_model` (+ inferred `adapter|merge|quantized|finetune` relation), `datasets` (creates "Datasets used to train" links), `new_version` (follows chain to latest), `model-index` eval results ([model-cards](https://github.com/huggingface/hub-docs/blob/main/docs/hub/model-cards.md)). Dataset card YAML: `license` keyword, `task_categories`, modality tags including **`3d`**, `audio`, `image`, `video`; sections recommended for bias ([datasets-cards](https://github.com/huggingface/hub-docs/blob/main/docs/hub/datasets-cards.md)).
- Data Studio: first 5 GB auto-converted to Parquet on branch `refs/convert/parquet`; `GET /api/datasets/{ns}/{name}/parquet` lists files; DuckDB SQL console; formats Parquet/CSV/WebDataset/JSONL ([data-studio](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/hub/data-studio.md)). This is a concrete example of a *derived, recomputable artifact kept on a side ref*.

### 1.4 Table-format snapshots: Delta, Iceberg, lakeFS, Lance

- **Delta Lake**: `_delta_log/<20-digit version>.json`, one action per line (`metaData`, `protocol`, `add`, `remove` with `deletionTimestamp`, `txn`, `commitInfo`); checkpoints (`n.checkpoint.parquet`, V2 UUID-named with sidecars) and `_last_checkpoint`; snapshot = replay in version order, latest reference per `(path, deletionVector.uniqueId)` wins; `remove` stays as a tombstone until expiry; vacuum after a retention period, default 7 days; deletion vectors mark soft-deleted rows ([PROTOCOL.md](https://raw.githubusercontent.com/delta-io/delta/master/PROTOCOL.md)).
- **Iceberg**: table state in metadata files; "an atomic swap of one table metadata file for another provides the basis for serializable isolation"; writers retry on conflict; snapshot → manifest list → manifests → data files (v4: single root manifest). **Refs**: tags (immutable labels) and branches (movable) each with `max-ref-age-ms` (never for `main`), `min-snapshots-to-keep`, `max-snapshot-age-ms`; expiry: drop aged non-main refs → keep referenced snapshots → keep branch ancestors within age/count → expire the rest; `snapshot-log` and `metadata-log` ([spec](https://raw.githubusercontent.com/apache/iceberg/main/format/spec.md)).
- **lakeFS**: Git-like repository over S3/Azure/GCS; zero-copy branches ("get a copy of the full production data, without copying anything"); S3-compatible API for Spark/Athena/DuckDB; hooks implement write-audit-publish ([README](https://github.com/treeverse/lakeFS)). Hook actions are YAML `on: pre-commit|pre-merge` with `branches:` filter and `hooks[]{id,type: webhook,properties{url,query_params}}`; shipped examples: file-format allowlist under `production/`, column-name disallow list (`user_`, `private_`) under `public/` to catch PII, dirty-partition check, required commit metadata (`airflow_dag_run_url`, `sources`) ([lakeFS-hooks](https://github.com/treeverse/lakeFS-hooks)). (Architecture page with Graveler/metarange internals was not reachable: [NOT FETCHED] https://docs.lakefs.io/latest/understand/architecture/.)
- **Lance**: manifest fields `fields` (schema), `fragments`, `version`, `timestamp` (UTC), `tag`, `index_section`, `writer_version`, `transaction_file` = `_transactions/{read_version}-{uuid}.txn`, `base_paths[]{id,name,is_dataset_root,path}` for shallow clones / imported files / multi-tier storage, `branch`, `next_row_id`, feature flags readers must refuse if unknown ([table.proto](https://raw.githubusercontent.com/lancedb/lance/main/protos/table.proto)); claims 100× faster random access than Parquet, blob encoding + lazy loading for images/video/audio, ACID + time travel + tags + branches with zero-copy versions, IVF_PQ vector index (< 1 ms/query on 1 M × 128-d SIFT, M2 laptop), "data evolution" (add columns without rewrite), integrations Arrow/Pandas/Polars/DuckDB/PyTorch/Spark/Ray/Trino/Flink ([README](https://github.com/lancedb/lance)).

*Lesson:* Curare's `reproducible` type should be modelled as an **Iceberg-style tag**: an immutable pointer to an immutable manifest, with explicit retention properties, and `working` as a branch with `min-snapshots-to-keep`. lakeFS hooks are the template for *review gates that run before a merge/release is allowed*.

### 1.5 Training-time file formats

- **safetensors**: bytes 0–7 = LE u64 header length N; N-byte JSON header `{tensor: {dtype, shape, data_offsets}}` + `__metadata__` (string→string only); buffer must be fully indexed with no gaps (blocks polyglot files); header capped at **100 MB** (DoS guard); lazy per-tensor load (BLOOM on 8 GPUs: ~10 min → ~45 s); pickle "runs arbitrary code" ([safetensors](https://github.com/huggingface/safetensors)). Any platform-side validator for model uploads should parse this header and reject `.pt/.pkl`.
- **Parquet**: `PAR1` magic, row groups (recommend **512 MB–1 GB**), pages (**8 KB**), footer-first metadata enabling column projection, CRC32 page checksums, min/max stats ([parquet-format](https://github.com/apache/parquet-format)).
- **WebDataset**: POSIX tar shards, "purely sequential I/O"; 3–10× higher I/O rate than random access on local drives; a sample = files sharing a basename; shard brace notation `{000000..012345}.tar`; `split_by_worker`/`split_by_node`; resampling for multi-node; reads via pipes (curl/aws cli), no metadata needed ([webdataset](https://github.com/webdataset/webdataset)).
- **MosaicML Streaming (MDS)**: zstd-compressed shards streamed from S3/GCS/Azure/OCI/Databricks to a local cache dir with `cache_limit` LRU eviction; deterministic, resumable mid-epoch shuffling independent of GPU/node/worker count; ImageNet/ResNet-50 after warm cache: 19k img/s vs ImageFolder 18k vs WebDataset 16k ([streaming](https://github.com/mosaicml/streaming)).

### 1.6 Scale reference points

| Object | Size / count | Source |
|---|---|---|
| ImageNet-1k (ILSVRC2012) | 1,281,167 train / 50,000 val / 100,000 test images; 155.84 GiB; two archives `ILSVRC2012_img_train.tar`, `ILSVRC2012_img_val.tar` | [TFDS catalog](https://raw.githubusercontent.com/tensorflow/datasets/master/docs/catalog/imagenet2012.md) |
| LAION-5B | 5.86 B URL/text pairs; 3 collections × 128 parquet parts; downloading at 384 px ≈ **220 TB** (≈ 50 TB at 256 px); ~1 week on 1 master + 10 × c6i.4xlarge at ~1,000 samples/s each; output as WebDataset shards of 10,000 samples; `--verify_hash` drops ~15 % (link rot) | [img2dataset laion5B](https://github.com/rom1504/img2dataset/blob/main/dataset_examples/laion5B.md) |
| Llama 3.1 | 8B / 70B / 405B; ~15 T tokens "publicly available sources"; cutoff Dec 2023; 39.3 M H100-80GB GPU-hours; custom community license | [MODEL_CARD](https://raw.githubusercontent.com/meta-llama/llama-models/main/models/llama3_1/MODEL_CARD.md) |
| DeepSeek-V3 | 671 B total (+14 B MTP = 685 B on the Hub), 37 B active, FP8 weights only, 128K context | [README](https://github.com/deepseek-ai/DeepSeek-V3) |
| Checkpoint arithmetic (derived) | 685 B params × 1 byte (FP8) ≈ 685 GB of weights; 405 B × 2 B (bf16) ≈ 810 GB; a full training checkpoint with fp32 master + 2 Adam moments ≈ 16 B/param ≈ 6.5 TB per save | derived from the two rows above |

### 1.7 GPU-cluster access patterns and why tiny files are the enemy

- **S3 behaviour**: per prefix ≥ 3,500 PUT/COPY/POST/DELETE and ≥ 5,500 GET/HEAD per second, unlimited prefixes (10 prefixes → 55k GET/s); first-byte latency **100–200 ms**; up to 100 Gb/s per instance; temporary `503 Slow Down` while partitions scale ([optimizing-performance](https://github.com/awsdocs/amazon-s3-userguide/blob/main/doc_source/optimizing-performance.md)). Design patterns: byte-range fetches at **8–16 MB granularity**, ~1 connection per 85–90 MB/s (≈ 15 connections for 10 Gb/s), retry small (< 512 KB) requests after 2 s then back off, retry the slowest 5 % of large transfers, new connection + fresh DNS on retry, CloudFront/ElastiCache for hot objects, Transfer Acceleration for cross-continent ([design-patterns](https://raw.githubusercontent.com/awsdocs/amazon-s3-userguide/main/doc_source/optimizing-performance-design-patterns.md)).
- **Derived cost of file-per-sample**: ImageNet as 1.28 M objects at 150 ms serial ≈ 53 h just in first-byte latency; with 64-way parallelism ≈ 50 min per epoch pass before any decode; as 128 shards of 10k samples (~1.2 GB each) the same read is ~128 range-streamed requests (derived from the S3 and TFDS numbers above).
- **FSx for Lustre**: throughput tiers e.g. PERSISTENT-125/250/500/1000 SSD = 320/640/1,300/2,600 MB/s per TiB network baseline; metadata ops sub-millisecond; data sub-ms (SSD) / single-digit ms (HDD); default stripe 1 MiB, progressive layout (1 stripe < 1 GiB, 5 stripes above), "large stripe counts on small files are suboptimal"; S3-imported files striped by `ImportedFileChunkSize` (default 1 GiB); backups unsupported on DRA-linked file systems ([performance](https://github.com/awsdocs/amazon-fsx-lustre-user-guide/blob/main/doc_source/performance.md)). (The lazy-load page was not reachable: [NOT FETCHED] https://docs.aws.amazon.com/fsx/latest/LustreGuide/.)
- **PyTorch ⇄ S3 connector**: `S3MapDataset` lists the whole prefix on first access ("can take time on large prefixes"); `S3IterableDataset` with `enable_sharding=True` per worker/host; sequential vs range-based readers (range useful ≥ 100 MB objects); Distributed Checkpoint `S3StorageWriter/Reader` with key prefix strategies (round-robin/binary/hex) **to avoid 503 throttling when many ranks write at once** ([s3-connector-for-pytorch](https://github.com/awslabs/s3-connector-for-pytorch)).
- **GPU-side decode**: DALI offloads decode/augment to GPU ("addresses the problem of the CPU bottleneck"), reads LMDB/RecordIO/TFRecord/COCO/JPEG/JPEG2000/WAV/FLAC/H.264/VP9/HEVC, supports GPUDirect Storage ([DALI](https://github.com/NVIDIA/DALI)); KvikIO exposes cuFile/GDS with POSIX fallback ([kvikio](https://github.com/rapidsai/kvikio)).
- **Local NVMe cache**: MDS `cache_limit` and Xet's chunk cache ("cache by range rather than individually, since neighbouring chunks are often needed together") are the two published cache policies ([streaming](https://github.com/mosaicml/streaming), [download](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/download-protocol.md)).

*Implication for Synergos:* a history node that "retains every version whole-file" is fine as the **system of record**, but training consumers need (a) an S3-compatible or range-GET surface with presigned, range-scoped URLs (Xet's `X-Xet-Signed-Range` is a good model for scoping custody URLs to byte ranges), (b) a sharded, sequential **export layout** (tar/MDS/parquet) produced as a derived artifact, and (c) a per-node NVMe cache keyed by content hash.

---

## 2. Reproducibility and lineage

### 2.1 Hash-pinned manifests

Every surveyed system pins by hash, but the *identity key* differs: DVC `.dir` = `[{md5, relpath}]`; LFS = sha256 + size; Croissant `cr:FileObject` carries `sha256` and `cr:RecordSet/Field` describe structure, adopted by HF, Kaggle, OpenML, TFDS ([croissant](https://github.com/mlcommons/croissant)); W&B reference artifacts store S3/GCS **ETag + size + versionId** and require bucket versioning or references break; default cap **10,000 objects** per reference (`max_objects=`), a new version is logged only when referenced files change ([track-external-files mirror](https://raw.githubusercontent.com/wandb/edu/main/llm-apps-course/docs_sample/track-external-files.md)); MLflow `Dataset{name, digest, source, schema, profile}` logged with `mlflow.log_input(ds, context="training")`, sources `s3://`, `delta://…@v1`, HTTP, local ([mlflow data](https://github.com/mlflow/mlflow/blob/master/docs/docs/classic-ml/dataset/index.mdx)).

### 2.2 Lineage graph vocabulary (OpenLineage)

`RunEvent{eventType: START|COMPLETE|FAIL|ABORT, eventTime, run{runId: UUID, facets}, job{namespace, name, facets}, inputs[], outputs[], producer, schemaURL}`; START plus one terminal event required. Facets: run `nominalTime, parent, errorMessage`; job `sourceCodeLocation, sourceCode, sql, ownership`; dataset `schema, dataSource, lifecycleStateChange, version, columnLineage, ownership`; input `dataQualityMetrics, dataQualityAssertions, inputStatistics`; output `outputStatistics`; re-emitting a facet name replaces it ([OpenLineage.md](https://github.com/OpenLineage/OpenLineage/blob/main/spec/OpenLineage.md)). `datasetVersion` is a required string ([facet](https://raw.githubusercontent.com/OpenLineage/OpenLineage/main/spec/facets/DatasetVersionDatasetFacet.json)); `dataQualityAssertions.assertions[]{assertion: not_null|unique|row_count|freshness|custom_sql, success, column}` ([facet](https://raw.githubusercontent.com/OpenLineage/OpenLineage/main/spec/facets/DataQualityAssertionsDatasetFacet.json)). Naming: S3 namespace `s3://{bucket}`, name `{object key}`; GCS `gs://{bucket}`; Azure `wasbs://{container}@{service}.dfs.core.windows.net`; local `file://{host}`; jobs `{dag_id}.{task_id}` etc. ([naming](https://raw.githubusercontent.com/OpenLineage/docs/main/docs/spec/naming.md)). *A Curare asset version maps cleanly to an OpenLineage dataset with namespace `curare://{org}/{project}` and `datasetVersion = manifest BLAKE3`.*

### 2.3 Model → dataset version → code commit → config

MLflow: each registered version links to the run/logged model that produced it; aliases are "a mutable, named reference to a particular version", more than one alias per version, `models:/name@alias`; stages (None/Staging/Production/Archived) **deprecated since 2.9.0** in favour of aliases + tags (e.g. `validation_status=passed`) + environment-prefixed registered models (`dev.`, `staging.`, `prod.`) with `copy_model_version()` for promotion; API `set_registered_model_alias(name, alias, version)`, `get_model_version_by_alias`, `delete_registered_model_alias`, `set_model_version_tag` ([registry](https://github.com/mlflow/mlflow/blob/master/docs/docs/classic-ml/model-registry/index.mdx), [workflow](https://raw.githubusercontent.com/mlflow/mlflow/master/docs/docs/classic-ml/model-registry/workflow.mdx)). HF model cards link `base_model` and `datasets` and chain `new_version` ([model-cards](https://github.com/huggingface/hub-docs/blob/main/docs/hub/model-cards.md)).

### 2.4 What to store vs recompute

Evidence-based rule: store **inputs + manifest + the exact transformation key**, recompute derived data through a cache keyed by that key. DVC run-cache keys outputs by hash(deps, params, cmd) ([DVC](https://github.com/iterative/dvc.org/blob/main/content/docs/user-guide/project-structure/internal-files.md)); Delta/Iceberg reconstruct state by replaying immutable actions ([Delta](https://raw.githubusercontent.com/delta-io/delta/master/PROTOCOL.md)); Lance records the transaction file per version and keeps per-version auxiliary data out of the default read path ([table.proto](https://raw.githubusercontent.com/lancedb/lance/main/protos/table.proto)); HF regenerates parquet previews on a side ref ([data-studio](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/hub/data-studio.md)). For Curare's `reproducible` artifact: persist the manifest, the recipe (tool id + version hash + params), and the environment digest; mark thumbnails, embeddings, parquet/MDS exports as *recomputable* with a TTL, and only pin them when a release references them.

### 2.5 Dedup and near-duplicate detection (layered)

1. Exact: content hash (BLAKE3/SHA-256) — free with CAS.
2. Sub-file: CDC chunks (Xet; see 1.2).
3. Perceptual images: PHash/DHash/WHash/AHash ("particularly good at finding exact duplicates", DHash fastest) and CNN embeddings for transformed near-duplicates ([imagededup](https://github.com/idealo/imagededup)).
4. Semantic: SemDeDup — embed (OpenCLIP), k-means, prune within-cluster pairs closer than `eps` ("lower epsilon will cause more aggressive pruning"); code archived Nov 2024 ([SemDeDup](https://github.com/facebookresearch/SemDeDup); paper numbers not fetchable: [NOT FETCHED] https://arxiv.org/abs/2303.09540).
5. Text: exact-substring via suffix array at 50 tokens ≈ 100 bytes, run twice ("cuts the number of duplicates down by over 100,000×"); one 50-word sequence repeats 60,000× in C4; NearDup clusters released for C4 (13 GB) ([deduplicate-text-datasets](https://github.com/google-research/deduplicate-text-datasets)).
6. Multimodal metadata-level: DataChain indexes buckets "no data copied, just typed metadata and file pointers", stores embeddings as `list[float]` columns and filters with `cosine_distance` + metadata, `delta=True` incremental re-runs ([datachain](https://github.com/iterative/datachain)).

### 2.6 Data quality gates

lakeFS pre-merge webhooks (format allowlist, schema/column disallow, required commit metadata) ([lakeFS-hooks](https://github.com/treeverse/lakeFS-hooks)); Great Expectations "unit tests for your data" with auto-generated validation docs ([GX](https://github.com/great-expectations/great_expectations)); results should be emitted as OpenLineage `dataQualityAssertions` so the gate outcome is part of lineage. Croissant RAI fields give the *schema* for what a gate should check: `rai:dataCollection`, `dataCollectionType`, `dataCollectionTimeframe`, `dataPreprocessingProtocol`, `dataAnnotationProtocol`, `personalSensitiveInformation`, `dataUseCases`, `dataLimitations`, `dataBiases`, `dataSocialImpact`, `annotatorDemographics`, `dataReleaseMaintenancePlan`; consent itself is not a defined field ([croissant-rai-spec](https://raw.githubusercontent.com/mlcommons/croissant/main/docs/croissant-rai-spec.md)).

---

## 3. Governance for AI training data in game studios

### 3.1 Per-asset licensing / consent / opt-out

The CAWG Training and Data Mining assertion (`cawg.training-mining`) is a map of entries `cawg.data_mining`, `cawg.ai_inference`, `cawg.ai_generative_training`, `cawg.ai_training` (generative and non-generative are separate "because generative AI training enables new assets to be created from training assets"), each with `use: allowed | notAllowed | constrained`; `constrained` means "contact the rights holder" and is treated as `notAllowed` absent more info, with free-text/URL `constraints_info`; custom keys allowed outside the reserved `cawg.` prefix; C2PA 1.4 used the legacy `c2pa.training-mining` label ([spec](https://raw.githubusercontent.com/decentralized-identity/cawg-training-and-data-mining-assertion/main/docs/modules/ROOT/partials/assertion-definition.adoc); c2pa-rs example sets both `ai_inference` and `ai_generative_training` to `notAllowed` ([cawg_identity.rs](https://raw.githubusercontent.com/contentauth/c2pa-rs/main/sdk/examples/cawg_identity.rs))). HF gating shows the *enforcement* half: per-user grant/revoke, audit export `{user, status, email, time, reviewedAt}`, geo-restriction flag ([models-gated](https://github.com/huggingface/hub-docs/blob/main/docs/hub/models-gated.md)).

### 3.2 Provenance of generated assets (prompt, model, seed) and C2PA

C2PA actions carry `digitalSourceType` as IPTC URIs, e.g. `http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia`; the SDK also defines `TrainedAlgorithmicData` (AI output "in a data format (e.g., CSV, pickle)"), `CompositeWithTrainedAlgorithmicMedia`, `DigitalCapture`, `CompositeCapture`, `Empty`; builder intents Create (adds `c2pa.created`), Edit (needs parent ingredient, adds `c2pa.opened`), Update ([intents](https://raw.githubusercontent.com/contentauth/c2pa-rs/main/docs/intents.md), [c2pa-rs](https://github.com/contentauth/c2pa-rs)). The c2pa-rs SDK uses v2 claims by default and supports JPEG/PNG/WebP/AVIF/HEIC/HEIF/JXL/DNG/GIF/TIFF/SVG/PDF/MP4/MOV/AVI/M4A/MP3/WAV/FLAC plus experimental text formats — **no glTF/GLB, FBX, USD, OBJ or texture-container formats** ([supported-formats](https://raw.githubusercontent.com/contentauth/c2pa-rs/main/docs/supported-formats.md)). Therefore Curare must hold a provenance record itself: `{generator: {tool, version}, model: {id, weights_hash}, prompt, negative_prompt, seed, sampler/params, source_ingredients: [asset_version_ids], digitalSourceType}`; for the 2D/video subset it can additionally embed a C2PA manifest via the SDK. DCC-side generation exists (Blender add-on rendering "an AI generated image based on a text prompt and your scene", local Automatic1111 or hosted backend) but exposes no standard metadata ([AI-Render](https://github.com/benrugg/AI-Render)), so capture must happen at the plugin/ingest boundary.

### 3.3 Watermarking

Invisible-watermark (dwtDct/dwtDctSvd/RivaGAN): 32–64-bit payloads; robust to JPEG, noise, brightness, overlay; **fails on 50 % resize, 30° rotation, aspect-changing crops**; dwtDct encode 300–350 ms at 1080p (only one suitable "on-the-fly") ([invisible-watermark](https://github.com/ShieldMnt/invisible-watermark)). SynthID-Text: logits-processor g-values from a key, Bayesian detector trained per key, reference code "not intended for production", hash not cryptographically secure ([synthid-text](https://github.com/google-deepmind/synthid-text)). Conclusion: watermarks are a *weak* signal; the strong signal is the signed catalog record (hash → provenance) — the platform should treat watermark presence as advisory metadata only.

### 3.4 PII / CSAM screening

Presidio: "context aware, pluggable and customizable PII de-identification service for text and images"; recognizers via NER, regex, rules, checksums; image redactor incl. DICOM; pip/Docker/Kubernetes/PySpark; warns it "may miss some sensitive information" ([presidio](https://github.com/microsoft/presidio)). PhotoDNA: perceptual "hash" matched against databases of previously identified illegal images, irreversible, "not facial recognition", free cloud service for qualified organisations, donated to NCMEC ([PhotoDNA](https://www.microsoft.com/en-us/photodna)). LAION's img2dataset keeps `NSFW`, `punsafe`, `pwatermark` and LICENSE columns but *does not filter on them by default* ([laion5B](https://github.com/rom1504/img2dataset/blob/main/dataset_examples/laion5B.md)) — the lesson is that screening must be an ingest-time **gate**, not a column. (Thorn Safer: [NOT FETCHED] https://safer.io/.)

### 3.5 EU AI Act (fetched from a community mirror of the OJ text; verify against EUR-Lex)

- Dates (Art. 113): in force 20 days after OJ publication; applies generally from **2 Aug 2026**; Chapters I–II from 2 Feb 2025; Ch. III §4, Ch. V (GPAI), VII, XII and Art. 78 from 2 Aug 2025; Art. 6(1) from 2 Aug 2027 ([Art. 113 mirror](https://raw.githubusercontent.com/mcekikj/eu-ai-act-knowledge-graph/main/EU-AI-Act-Vault/01%20Articles/Article%20113.md); official text: [NOT FETCHED] https://eur-lex.europa.eu/eli/reg/2024/1689/oj). Any "Digital Omnibus" postponement of high-risk deadlines could not be verified from this sandbox.
- Art. 10 (high-risk): training/validation/testing sets must follow governance practices covering (a) design choices, (b) "data collection processes and the origin of data" incl. original purpose of personal data, (c) preparation (annotation, labelling, cleaning, updating, enrichment, aggregation), (d) assumptions, (e) availability/quantity/suitability, (f) bias examination, (g) bias mitigation, (h) gaps; sets must be "relevant, sufficiently representative, and to the best extent possible, free of errors and complete"; special-category data for bias correction only with strict access control, no onward transfer, and **deletion once bias is corrected or retention ends** ([Art. 10 mirror](https://raw.githubusercontent.com/mcekikj/eu-ai-act-knowledge-graph/main/EU-AI-Act-Vault/01%20Articles/Article%2010.md)).
- Art. 50: providers of systems generating synthetic audio/image/video/text "shall ensure that the outputs of the AI system are marked in a machine-readable format" with solutions "effective, interoperable, robust and reliable"; deployers of deep fakes must disclose; AI Office to foster codes of practice ([Art. 50 mirror](https://raw.githubusercontent.com/mcekikj/eu-ai-act-knowledge-graph/main/EU-AI-Act-Vault/01%20Articles/Article%2050.md)).
- Art. 53(1) GPAI providers: technical documentation (Annex XI), downstream information (Annex XII), a copyright policy that identifies and respects **Art. 4(3) DSM Directive rights reservations**, and a public "sufficiently detailed summary about the content used for training" on the AI Office template ([Art. 53 mirror](https://raw.githubusercontent.com/mcekikj/eu-ai-act-knowledge-graph/main/EU-AI-Act-Vault/01%20Articles/Article%2053.md); template: [NOT FETCHED] https://digital-strategy.ec.europa.eu/en/library/explanatory-notice-and-template-public-summary-training-content-general-purpose-ai-models).

### 3.6 Japan

Could not be fetched from this sandbox: Copyright Act Art. 30-4 (information-analysis exception with the "unreasonably prejudice the interests of the copyright owner" proviso) — [NOT FETCHED] https://www.japaneselawtranslation.go.jp/en/laws/view/4207 and https://laws.e-gov.go.jp/law/345AC0000000048; the Agency for Cultural Affairs "General Understanding on AI and Copyright" — [NOT FETCHED] https://www.bunka.go.jp/english/policy/copyright/pdf/94055801_01.pdf; the 2025 AI Promotion Act — no reachable canonical URL. Design consequence that does not depend on the exact text: the platform must record, per asset, (i) the rights holder, (ii) any machine-readable rights reservation, (iii) the intended *purpose* of a dataset build, so counsel can apply whichever jurisdictional test (EU Art. 4(3) reservation, JP Art. 30-4 proviso) at build time.

### 3.7 Storefront and studio policies

Steam's AI-content disclosure (pre-generated vs live-generated, content survey, store-page display) — [NOT FETCHED] https://store.steampowered.com/news/group/4145017/view/3862463747997849618 and https://partner.steamgames.com/doc/gettingstarted/contentsurvey. Square Enix / Capcom / Nexon statements — not reachable (e.g. [NOT FETCHED] https://www.hd.square-enix.com/eng/news/2024/html/a_new_years_letter_from_the_president.html). What any such policy needs from the platform is the same: a **per-build export listing every shipped asset whose provenance chain contains `trainedAlgorithmicMedia`, with the generator/model identity**, which is derivable from the §3.2 record.

### 3.8 Retention and deletion

Deletion duties interact badly with "history nodes retain every version": Art. 10(5)(e) requires deleting special-category data once bias is corrected ([Art. 10 mirror](https://raw.githubusercontent.com/mcekikj/eu-ai-act-knowledge-graph/main/EU-AI-Act-Vault/01%20Articles/Article%2010.md)); GDPR Art. 17 erasure — [NOT FETCHED] https://eur-lex.europa.eu/eli/reg/2016/679/oj. Table formats show the accepted mechanics: Delta tombstones + vacuum after 7 days ([Delta](https://raw.githubusercontent.com/delta-io/delta/master/PROTOCOL.md)); Iceberg ref/snapshot retention properties ([Iceberg](https://raw.githubusercontent.com/apache/iceberg/main/format/spec.md)); HF commit/blob removal is out of scope here. The platform needs: a **tombstone** in the manifest history (hash stays, bytes go), per-object legal-hold and retention class, and derived-artifact invalidation (embeddings, thumbnails, exported shards) when a source is erased.

---

## 4. Search and discovery

- **Metadata/tag model**: HF uses a flat `tags[]` plus typed fields (`license`, `task_categories`, modality incl. `3d`, `library_name`, `base_model`, `datasets`) ([datasets-cards](https://github.com/huggingface/hub-docs/blob/main/docs/hub/datasets-cards.md), [model-cards](https://github.com/huggingface/hub-docs/blob/main/docs/hub/model-cards.md)); Croissant adds structured RAI fields ([RAI](https://raw.githubusercontent.com/mlcommons/croissant/main/docs/croissant-rai-spec.md)). Curare should separate *controlled* fields (license, rights bits, modality, provenance type) from free tags.
- **Embedding search**: CLIP `encode_image`/`encode_text`, logits = cosine × 100, MIT licence, larger OpenCLIP models up to ViT-G/14 ([CLIP](https://github.com/openai/CLIP)); 3D: OpenShape aligns point-cloud encoders with OpenCLIP ViT-bigG-14 image/text features trained on Objaverse/ShapeNet/3D-FUTURE/ABO (~205 GB), enabling text→3D retrieval ([OpenShape](https://github.com/Colin97/OpenShape_code)); storage/query: Lance IVF_PQ + BM25 + SQL hybrid ([Lance](https://github.com/lancedb/lance)), DataChain embeddings-as-columns ([DataChain](https://github.com/iterative/datachain)).
- **Thumbnails/previews**: `<model-viewer>` renders glTF/GLB in-browser and its fidelity tests capture Puppeteer screenshots but need a display ([model-viewer](https://github.com/google/model-viewer)); glTF is the royalty-free runtime format that "minimizes both the size of 3D assets, and the runtime processing" ([glTF](https://github.com/KhronosGroup/glTF)). Commercial DAM behaviour (Helix DAM, Bynder, AEM 3D) could not be fetched ([NOT FETCHED] https://www.perforce.com/products/helix-dam, https://experienceleague.adobe.com/en/docs/experience-manager-cloud-service/content/assets/manage/3d-assets).

---

## 5. Workflow integration

- **Consumers**: training jobs read sharded, sequential formats from S3-compatible endpoints or a Lustre/NVMe cache (§1.5–1.7); inference loads safetensors lazily by header (§1.5); DCC plugins generate at the artist's desk and need an ingest hook (§3.2).
- **Dataset snapshot as immutable release**: Iceberg *tag* with retention ([Iceberg](https://raw.githubusercontent.com/apache/iceberg/main/format/spec.md)); lakeFS commit on a protected branch gated by pre-merge hooks ([lakeFS-hooks](https://github.com/treeverse/lakeFS-hooks)); Lance `tag` field ([table.proto](https://raw.githubusercontent.com/lancedb/lance/main/protos/table.proto)); W&B alias `latest`/custom ([W&B](https://raw.githubusercontent.com/wandb/edu/main/llm-apps-course/docs_sample/track-external-files.md)); HF side-ref `refs/convert/parquet` for the training-ready representation ([data-studio](https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/hub/data-studio.md)).
- **Feature-store analogy**: Feast separates an offline store (training, `get_historical_features` with point-in-time-correct joins so "future feature values" don't leak) from an online store (`materialize`, `get_online_features`), with `feast apply` registering definitions ([Feast](https://github.com/feast-dev/feast)). The analogue: Curare catalog = registry; release manifest = offline snapshot; CDN/edge cache = online store; "point-in-time" = resolve every reference by manifest hash at build time.
- **Registry promotion ↔ submission/review/release**: MLflow's recommended model is *versions + mutable aliases + status tags + per-environment registered models + `copy_model_version`* ([workflow](https://raw.githubusercontent.com/mlflow/mlflow/master/docs/docs/classic-ml/model-registry/workflow.mdx)). Mapping: Curare `working` = branch/dev model; `submission` = new immutable version with `validation_status=pending`; `review` = lakeFS-style pre-merge gates writing `dataQualityAssertions`; `release` = alias (`champion`, `prod`) / tag with retention; rollback = re-point alias.

---

## 6. AI-dev need → capability → minimal data-model / API implication

| AI-dev need | Concrete capability the Synergos/Curare platform must provide | Minimal data-model / API implication |
|---|---|---|
| Pin a training set exactly | Immutable **manifest object** (ordered `{path, blake3, size, mtime?}`), addressed by its own BLAKE3, referenced from git | `manifest{hash, entries[], created_at, parent?}`; `GET /manifests/{hash}`; git stores only the hash (DVC `.dir` / Iceberg manifest-list pattern) |
| Snapshot vs working data | **Refs with retention**: tags (immutable) and branches (movable) carrying `max-ref-age`, `min-snapshots-to-keep` | `ref{name, kind: tag\|branch, manifest_hash, retention{...}}`; GC = Iceberg 4-step expiry |
| Train from object storage at line rate | S3-compatible / range-GET surface with **range-scoped presigned URLs**, 8–16 MB ranges, per-file chunk-length table | custody URL grants `{object, byte_ranges[], expires}` (Xet `X-Xet-Signed-Range` model); store `chunk_lengths[]` per blob |
| Avoid tiny-file I/O | **Derived export layouts**: tar/WebDataset, MDS, parquet (512 MB–1 GB row groups) built from a manifest | `export{manifest_hash, format, shard_size, status, output_manifest_hash}` as a recomputable job; cache keyed by (manifest_hash, format, tool_version) |
| Cheap checkpoint iteration | Either CDC dedup (64 KiB chunks / 64 MiB blocks, 30–85 % savings) or at least chunk tables enabling later migration | keep file hash as Merkle root over fixed-policy chunks even if transfer stays whole-file |
| Lineage model→data→code→config | Emit **OpenLineage** RunEvents with `datasetVersion = manifest hash`, `sourceCodeLocation = git sha`, params in job facets | `run{run_id uuid, job, inputs[{curare://…, version}], outputs[], facets}`; `POST /lineage/events`; query "what trained on asset X" |
| Reproducible derived artifacts | **Run-cache**: outputs keyed by hash(inputs manifest, params, tool+version) | `recipe{hash, inputs[], params, tool{id, version_hash}}`, `result{recipe_hash → output_manifest_hash}` |
| Dataset dedup / near-dup | Pluggable **dedup pipeline**: exact → chunk → perceptual hash → embedding (SemDeDup `eps`) | `fingerprint{asset_version, kind: phash\|dhash\|clip\|openshape, vector\|hash}`; similarity query with filters |
| Quality gates before release | **Pre-merge/pre-release hooks** (webhook) returning assertions; failures block the transition | `gate{on: submission\|release, hooks[]}`; results stored as `dataQualityAssertions[]{assertion, success, column?}` |
| Licence / consent / opt-out per asset | **Rights record** with CAWG entries | `rights{holder, license, cawg: {data_mining, ai_inference, ai_generative_training, ai_training} ∈ allowed\|notAllowed\|constrained, constraints_info, reservation_uri}`; dataset build rejects assets whose bits conflict with build purpose |
| Provenance of generated assets | **Provenance record** incl. `digitalSourceType`, generator, model hash, prompt, seed, ingredients; C2PA embed for 2D/video only | `provenance{digital_source_type, generator{tool, version}, model{id, weights_hash}, prompt, seed, params, ingredients[asset_version_ids], c2pa_manifest?}` |
| Regulatory documentation (AI Act Art. 10/53, Croissant RAI) | **Dataset card** generated from catalog fields; export Croissant JSON-LD + training-content summary | `dataset{croissant_rai{...}, intended_use, collection_timeframe, preprocessing, known_biases, maintenance_plan}`; `GET /datasets/{id}/croissant` |
| Screening (PII/CSAM/NSFW) | Ingest-time **scan gate** calling Presidio/PhotoDNA-class services; results immutable and auditable | `scan{asset_version, scanner, version, verdict, evidence_hash, at}`; quarantine state on asset version |
| Deletion / retention / legal hold | **Tombstones** in history, retention class per object, hold flag, derived-artifact invalidation | `blob{retention_class, legal_hold, tombstoned_at}`; manifest keeps hash; GC refuses held objects |
| Access control for datasets | **Gating** with per-user grants, audit export, geo flags | `grant{subject, dataset, status, reviewed_by, at}` via Cernere; export `{user, status, time, reviewedAt}` |
| Promotion stages | **Aliases + status tags** instead of fixed stages | `alias{model_or_dataset, name, version}` mutable; `tag{version, key, value}`; `POST /aliases` moves pointer |
| Search | Hybrid **metadata + vector** index over controlled fields and fingerprints | `embedding{asset_version, model_id, vector}`; query `{text?, image?, filters}` |

---

## 7. Things NOT to build (integrate instead)

- **Experiment tracking / model registry UI** — MLflow or W&B already define versions, aliases, lineage and dataset digests; expose Curare manifests as MLflow `DatasetSource` / W&B reference artifacts (ETag/size/versionId) instead ([MLflow](https://github.com/mlflow/mlflow/blob/master/docs/docs/classic-ml/dataset/index.mdx), [W&B](https://raw.githubusercontent.com/wandb/edu/main/llm-apps-course/docs_sample/track-external-files.md)).
- **A lineage graph store** — emit OpenLineage events; let Marquez/Atlan/whatever store them ([OpenLineage](https://github.com/OpenLineage/OpenLineage/blob/main/spec/OpenLineage.md)).
- **Columnar/table engines** — Parquet/Delta/Iceberg/Lance for tabular and multimodal tables; Curare should reference their snapshot ids, not re-implement row-level versioning ([Iceberg](https://raw.githubusercontent.com/apache/iceberg/main/format/spec.md), [Lance](https://github.com/lancedb/lance)).
- **Training data loaders / GPU decode** — WebDataset, MosaicML Streaming, S3 connector for PyTorch, DALI; the platform's job ends at producing shards and range-readable endpoints ([streaming](https://github.com/mosaicml/streaming), [s3-connector](https://github.com/awslabs/s3-connector-for-pytorch), [DALI](https://github.com/NVIDIA/DALI)).
- **Vector databases** — Lance/LanceDB, DataChain or a hosted vector store; the platform stores fingerprints and calls out ([Lance](https://github.com/lancedb/lance), [DataChain](https://github.com/iterative/datachain)).
- **PII/CSAM detectors and watermark algorithms** — Presidio, PhotoDNA (and Thorn-class services), SynthID/invisible-watermark; treat as pluggable scanners ([Presidio](https://github.com/microsoft/presidio), [PhotoDNA](https://www.microsoft.com/en-us/photodna)).
- **C2PA signing stack** — use c2pa-rs (Rust, fits Synergos) for the formats it supports; do not write a manifest signer ([c2pa-rs](https://github.com/contentauth/c2pa-rs)).
- **Data-quality rule engines** — Great Expectations or SQL assertions behind a webhook gate ([GX](https://github.com/great-expectations/great_expectations), [lakeFS-hooks](https://github.com/treeverse/lakeFS-hooks)).
- **A parallel file system** — FSx for Lustre / Weka / local NVMe caches are the consumers' problem; provide the S3-compatible surface and let them lazy-load ([FSx perf](https://github.com/awsdocs/amazon-fsx-lustre-user-guide/blob/main/doc_source/performance.md)).

---

## 8. Gaps / items to verify before design sign-off

- Official texts: EU AI Act (EUR-Lex), GDPR Art. 17, Japan Copyright Act Art. 30-4 and the 2025 AI Promotion Act, Steam content-survey rules, Square Enix / Capcom / Nexon statements — all **unreachable from this sandbox**; EU articles were read from a community mirror and must be re-checked.
- No fetched source gives CDC dedup ratios for **game asset formats** (DDS/KTX2 textures, FBX/USD meshes, Wwise banks); the 30–85 % Xet figures are for ML checkpoints and GGUF quantizations only.
- FSx Lustre lazy-load mechanics, Thorn Safer capabilities, Helix DAM/Bynder/AEM 3D indexing behaviour, SemDeDup paper figures, Datasheets/Model-Cards papers were not fetchable.
- Azure ML / GCS equivalents of the S3 numbers were not fetched; the S3 figures are AWS-specific.

---

## Sources (fetched unless marked)

1. Hugging Face Xet dedup spec — https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/deduplication.md
2. Xet hashing spec — https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/hashing.md
3. Xet upload protocol — https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/upload-protocol.md
4. Xet download protocol — https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/download-protocol.md
5. Xet xorb format — https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/xorb.md
6. Xet intro page — https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/hub/xet/index.md
7. xet-core repo (constants, crates) — https://github.com/huggingface/xet-core
8. HF blog "From files to chunks" — https://raw.githubusercontent.com/huggingface/blog/main/from-files-to-chunks.md
9. HF blog "From chunks to blocks" — https://raw.githubusercontent.com/huggingface/blog/main/from-chunks-to-blocks.md
10. HF blog "Xet on the Hub" — https://raw.githubusercontent.com/huggingface/blog/main/xet-on-the-hub.md
11. HF storage limits — https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/hub/storage-limits.md
12. HF gated models — https://github.com/huggingface/hub-docs/blob/main/docs/hub/models-gated.md
13. HF model cards — https://github.com/huggingface/hub-docs/blob/main/docs/hub/model-cards.md
14. HF dataset cards — https://github.com/huggingface/hub-docs/blob/main/docs/hub/datasets-cards.md
15. HF Data Studio / parquet conversion — https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/hub/data-studio.md
16. Git LFS spec — https://github.com/git-lfs/git-lfs/blob/main/docs/spec.md
17. DVC internal files — https://github.com/iterative/dvc.org/blob/main/content/docs/user-guide/project-structure/internal-files.md
18. DVC .dvc files — https://github.com/iterative/dvc.org/blob/main/content/docs/user-guide/project-structure/dvc-files.md
19. DVC dvc.yaml / dvc.lock — https://raw.githubusercontent.com/iterative/dvc.org/main/content/docs/user-guide/project-structure/dvcyaml-files.md
20. DataChain — https://github.com/iterative/datachain
21. lakeFS README — https://github.com/treeverse/lakeFS
22. lakeFS hooks examples — https://github.com/treeverse/lakeFS-hooks
23. Delta Lake protocol — https://raw.githubusercontent.com/delta-io/delta/master/PROTOCOL.md
24. Apache Iceberg table spec — https://raw.githubusercontent.com/apache/iceberg/main/format/spec.md
25. Lance table.proto — https://raw.githubusercontent.com/lancedb/lance/main/protos/table.proto
26. Lance README — https://github.com/lancedb/lance
27. safetensors — https://github.com/huggingface/safetensors
28. Parquet format — https://github.com/apache/parquet-format
29. WebDataset — https://github.com/webdataset/webdataset
30. MosaicML Streaming — https://github.com/mosaicml/streaming
31. TFDS imagenet2012 — https://raw.githubusercontent.com/tensorflow/datasets/master/docs/catalog/imagenet2012.md
32. img2dataset LAION-5B example — https://github.com/rom1504/img2dataset/blob/main/dataset_examples/laion5B.md
33. Llama 3.1 model card — https://raw.githubusercontent.com/meta-llama/llama-models/main/models/llama3_1/MODEL_CARD.md
34. DeepSeek-V3 README — https://github.com/deepseek-ai/DeepSeek-V3
35. S3 performance — https://github.com/awsdocs/amazon-s3-userguide/blob/main/doc_source/optimizing-performance.md
36. S3 performance design patterns — https://raw.githubusercontent.com/awsdocs/amazon-s3-userguide/main/doc_source/optimizing-performance-design-patterns.md
37. FSx for Lustre performance — https://github.com/awsdocs/amazon-fsx-lustre-user-guide/blob/main/doc_source/performance.md
38. S3 connector for PyTorch — https://github.com/awslabs/s3-connector-for-pytorch
39. NVIDIA DALI — https://github.com/NVIDIA/DALI
40. KvikIO (GPUDirect Storage bindings) — https://github.com/rapidsai/kvikio
41. OpenLineage object model — https://github.com/OpenLineage/OpenLineage/blob/main/spec/OpenLineage.md
42. OpenLineage naming — https://raw.githubusercontent.com/OpenLineage/docs/main/docs/spec/naming.md
43. OpenLineage datasetVersion facet — https://raw.githubusercontent.com/OpenLineage/OpenLineage/main/spec/facets/DatasetVersionDatasetFacet.json
44. OpenLineage dataQualityAssertions facet — https://raw.githubusercontent.com/OpenLineage/OpenLineage/main/spec/facets/DataQualityAssertionsDatasetFacet.json
45. MLflow model registry — https://github.com/mlflow/mlflow/blob/master/docs/docs/classic-ml/model-registry/index.mdx
46. MLflow registry workflow (aliases, stage deprecation) — https://raw.githubusercontent.com/mlflow/mlflow/master/docs/docs/classic-ml/model-registry/workflow.mdx
47. MLflow dataset tracking — https://github.com/mlflow/mlflow/blob/master/docs/docs/classic-ml/dataset/index.mdx
48. W&B reference artifacts (mirror of docs page in wandb/edu) — https://raw.githubusercontent.com/wandb/edu/main/llm-apps-course/docs_sample/track-external-files.md
49. Croissant — https://github.com/mlcommons/croissant
50. Croissant RAI spec — https://raw.githubusercontent.com/mlcommons/croissant/main/docs/croissant-rai-spec.md
51. Feast — https://github.com/feast-dev/feast
52. Great Expectations — https://github.com/great-expectations/great_expectations
53. imagededup — https://github.com/idealo/imagededup
54. SemDeDup code — https://github.com/facebookresearch/SemDeDup
55. Deduplicating text datasets — https://github.com/google-research/deduplicate-text-datasets
56. CLIP — https://github.com/openai/CLIP
57. OpenShape — https://github.com/Colin97/OpenShape_code
58. model-viewer — https://github.com/google/model-viewer
59. glTF — https://github.com/KhronosGroup/glTF
60. AI-Render (Blender SD add-on) — https://github.com/benrugg/AI-Render
61. CAWG training & data-mining assertion — https://raw.githubusercontent.com/decentralized-identity/cawg-training-and-data-mining-assertion/main/docs/modules/ROOT/partials/assertion-definition.adoc
62. c2pa-rs README — https://github.com/contentauth/c2pa-rs
63. c2pa-rs supported formats — https://raw.githubusercontent.com/contentauth/c2pa-rs/main/docs/supported-formats.md
64. c2pa-rs intents / digitalSourceType — https://raw.githubusercontent.com/contentauth/c2pa-rs/main/docs/intents.md
65. c2pa-rs CAWG example — https://raw.githubusercontent.com/contentauth/c2pa-rs/main/sdk/examples/cawg_identity.rs
66. invisible-watermark — https://github.com/ShieldMnt/invisible-watermark
67. SynthID-Text — https://github.com/google-deepmind/synthid-text
68. Microsoft Presidio — https://github.com/microsoft/presidio
69. Microsoft PhotoDNA — https://www.microsoft.com/en-us/photodna
70. EU AI Act Art. 10 (community mirror of OJ text) — https://raw.githubusercontent.com/mcekikj/eu-ai-act-knowledge-graph/main/EU-AI-Act-Vault/01%20Articles/Article%2010.md
71. EU AI Act Art. 50 (mirror) — https://raw.githubusercontent.com/mcekikj/eu-ai-act-knowledge-graph/main/EU-AI-Act-Vault/01%20Articles/Article%2050.md
72. EU AI Act Art. 53 (mirror) — https://raw.githubusercontent.com/mcekikj/eu-ai-act-knowledge-graph/main/EU-AI-Act-Vault/01%20Articles/Article%2053.md
73. EU AI Act Art. 113 (mirror) — https://raw.githubusercontent.com/mcekikj/eu-ai-act-knowledge-graph/main/EU-AI-Act-Vault/01%20Articles/Article%20113.md

Not fetched (canonical URLs, verify separately): EU AI Act OJ text https://eur-lex.europa.eu/eli/reg/2024/1689/oj; GDPR https://eur-lex.europa.eu/eli/reg/2016/679/oj; AI Office training-summary template https://digital-strategy.ec.europa.eu/en/library/explanatory-notice-and-template-public-summary-training-content-general-purpose-ai-models; Japan Copyright Act https://www.japaneselawtranslation.go.jp/en/laws/view/4207 and https://laws.e-gov.go.jp/law/345AC0000000048; ACA AI & Copyright guidance https://www.bunka.go.jp/english/policy/copyright/pdf/94055801_01.pdf; Steam AI disclosure https://store.steampowered.com/news/group/4145017/view/3862463747997849618 and https://partner.steamgames.com/doc/gettingstarted/contentsurvey; lakeFS architecture https://docs.lakefs.io/latest/understand/architecture/; FSx Lustre guide https://docs.aws.amazon.com/fsx/latest/LustreGuide/; Thorn Safer https://safer.io/; Helix DAM https://www.perforce.com/products/helix-dam; AEM 3D assets https://experienceleague.adobe.com/en/docs/experience-manager-cloud-service/content/assets/manage/3d-assets; SemDeDup paper https://arxiv.org/abs/2303.09540; "Git is for Data" (CIDR 2023) https://www.cidrdb.org/cidr2023/papers/p43-low.pdf.
