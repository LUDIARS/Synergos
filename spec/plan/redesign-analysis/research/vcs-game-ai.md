# Version control for game studios and AI teams: how the incumbents solve the problems

Research notes for the Synergos + Curare redesign. Date: 2026-10-15.
Scope: Perforce Helix Core (P4), Unity Version Control (Plastic SCM), Git LFS and the Git-adjacent tools (partial clone, git-annex, Diversion, Anchorpoint, Snowtrack/P4 One, Alienbrain, SVN), Helix DAM, Unreal Engine's data pipeline (OFPA, Virtual Assets, Zen, Cloud DDC, Horde), and the ML-data side (DVC, lakeFS, Hugging Face Xet, Pachyderm, Dolt, W&B Artifacts, MLflow, Oxen, Delta/Iceberg, dataset cards).

Every claim carries a URL. Where a number or behaviour could not be confirmed from a primary source it is marked **unverified**. Fetch access in this session was limited to github.com, so vendor-hosted pages were read through search snippets; the URLs are the canonical pages and should be re-read before a design decision depends on an exact number.

Baseline assumptions about our stack (from the task context, not re-verified here): Synergos = Rust, P2P QUIC transfer, history nodes retaining every version whole-file, content-addressed by BLAKE3, Cloudflare tunnel/relay, control plane, custody service with HMAC-signed PUT/GET URLs and SHA-256-verified receipts; Curare = TypeScript/Hono/PostgreSQL/MinIO asset catalog with versioning types working/submission/reproducible, submissions/review/release, access policy via identity service "Cernere". Decision 2026-08-16: no delta/CDC transfer; git holds text + a manifest of pointers; Synergos moves blobs.

---

## 1. Perforce Helix Core (P4)

### 1.1 What it solves and for whom
P4 is the de facto AAA standard: Perforce claims 19 of the top 20 AAA studios and that Epic uses it for Unreal Engine development ([perforce.com/products/helix-core](https://www.perforce.com/products/helix-core), vendor claim). It serves artists (exclusive checkout, simple sync/submit), engineers (streams, changelists, atomic submits) and build farms (edge/proxy topologies) from one server with one permissions model.

### 1.2 Mechanisms

**Streams.** Branches with declared stability and a parent/child graph: mainline (no parent), release streams (more stable than parent, expect merge-down), development and task streams (less stable, expect copy-up). "Merge down before copy up" is enforced by the stream model ([P4V streams doc](https://www.perforce.com/manuals/v23.2/p4v/Content/P4V/streams.about.html), [p4 stream cmdref](https://ftp.perforce.com/perforce/r19.1/doc/manuals/cmdref/Content/CmdRef/p4_stream.html)). Stream specs are themselves protected objects with `readstreamspec` / `openstreamspec` / `writestreamspec` rights ([p4 protect cmdref](https://perforce.com/manuals/cmdref/Content/CmdRef/p4_protect.html)).

**Exclusive open (`+l`) and typemap.** P4 allows concurrent edits by default; pessimistic locking is a per-file *type modifier*, `+l`, that the `p4 typemap` table applies automatically to newly added files matching a path pattern (e.g. `binary+l //depot/....uasset`). Modifiers stack: `binary+lFS10` = exclusive, stored full (not compressed), keep only last 10 revisions. Typemap applies only to new adds; existing files need `p4 edit -t+l` / `p4 reopen -t+l`; `-t` on the command line overrides typemap ([p4 typemap cmdref](https://perforce.com/manuals/cmdref/Content/CmdRef/p4_typemap.html), [Configuring the server](https://www.perforce.com/manuals/v16.1/p4sag/chapter.configuring.html)).

**Large-file storage.** Text revisions are RCS deltas by default; binary revisions are stored whole and compressed (`+C`) by default; `+F` stores full and uncompressed (recommended for already-compressed media: .jpg/.mpg/.avi); text files larger than `filetype.maxtextsize` (default 10 MB) are auto-typed `text+C` and stored whole ([file types](https://www.perforce.com/manuals/cmdref/Content/CmdRef/file.types.html), [modifiers](https://www.perforce.com/manuals/v17.1/cmdref/Content/P4Guide/filetypes.modifiers.html), [storage](https://www.perforce.com/manuals/v17.1/cmdref/Content/P4Guide/filetypes.storage.html), [compression guidance](https://perforce.com/manuals/p4sag/Content/P4SAG/performance.prevention.compression.html)). `+Sn` caps retained revisions per file: this is P4's answer to "every version whole-file" storage growth.

**Protections table (`p4 protect`).** One ordered table, five fields per line: *access level, user|group, name, host (IP / CIDR, comma-separated since 2021.1), depot path pattern*. Levels: `list < read < open < write < review < admin < super`, plus `owner` (delegated `p4 protect` on a sub-path) and the exact-right forms `=read/=open/=write/=branch` that grant or deny a single right without implying lesser ones. A leading `-` on the path denies; a denial overrides any grant for matching files/hosts. Until the first `p4 protect` run everyone is super ([p4 protect r16.1](https://www.perforce.com/perforce/r16.1/manuals/cmdref/p4_protect.html), [protections chapter](https://www.perforce.com/manuals/v15.1/p4sag/chapter.protections.html), [current p4 protect](https://perforce.com/manuals/cmdref/Content/CmdRef/p4_protect.html), [setting protections](https://www.perforce.com/manuals/v20.2/p4sag/Content/P4SAG/protections.set.html)). Granularity = path pattern x principal x source IP, which is finer than most Git hosts (repo/branch) and is the model game studios are used to.

**Obliterate.** `p4 obliterate` removes file revisions *and* their metadata, label and client references; nothing happens without `-y` (dry run by default); requires super/admin; `-z` only undoes lazy copies ([obliterate cmdref](https://perforce.com/perforce/doc.092/manuals/cmdref/obliterate.html)). This is the legal/DMCA/leak-removal primitive that an append-only CAS must still provide.

**Topologies.**
- *Helix Proxy (p4p)*: a transparent file-content cache between clients and the server; keeps revisions in `P4PCACHE`, populated lazily on sync/submit (prefill by syncing a dummy workspace), `-S` disables cache-fault coordination, free to run, needs server >= 2002.2 (SSL >= 2012.1) ([Helix Proxy](https://www.perforce.com/manuals/p4sag/Content/P4SAG/chapter.proxy.html), [preloading the cache](https://ftp.perforce.com/perforce/r18.2/doc/manuals/p4dist/Content/P4Dist/proxy.performance.preload_cache.html), [2019.2 proxy chapter](https://www.perforce.com/manuals/p4dist/Content/P4Dist/chapter.proxy.html)).
- *Forwarding replica*: a replica (metadata + files) that serves reads locally and forwards writes to the master; "a blend of the Helix Proxy with the improved performance of a replica"; needs its own `P4AUDIT` log; until 2021.2 could not sit between an edge and commit server ([forwarding replica](https://www.perforce.com/manuals/p4sag/Content/P4SAG/forwarding-replica.html)).
- *Commit/edge*: commit server owns submits, global workspaces and promoted shelves; edge servers (created from a commit checkpoint) run most commands locally, so this gives "best overall performance", but each edge must be provisioned and backed up and cannot serve as standby; `p4 unsubmit/resubmit` are commit-only. Build-only edges need no backup ([Commit-edge overview](https://www.perforce.com/manuals/p4sag/Content/P4SAG/Commit-edge-overview.html), [deployment architecture](https://perforce.com/manuals/v23.1/p4sag/Content/P4SAG/deployment-architecture.html)). Commit-edge shipped in 2013.2 ([Assembla comparison](https://blog.assembla.com/the-top-5-pros-and-cons-of-perforce-helix-core-edge-servers-vs-forwarding-replicas), vendor-affiliated).
- *Federated services*: all of the above are composed from the same replication machinery (checkpoints + journal pull), with SSL key management as a prerequisite ([Commit-edge overview](https://www.perforce.com/manuals/p4sag/Content/P4SAG/Commit-edge-overview.html)).

**Authentication.** Native LDAP specs are preferred over scripts; SSO via `auth-check-sso` triggers (client uses `P4LOGINSSO`); MFA needs three triggers (`auth-pre-2fa`, `auth-init-2fa`, `auth-check-2fa`), forces security level >= 3, and is enabled per user via `AuthMethod: perforce+2fa|ldap+2fa` + `p4 login2`. Cautions in the docs: a misspelled trigger name locks everyone out; an auth-check trigger disables built-in password-strength rules; passwords are passed to triggers on stdin, never argv ([MFA](https://perforce.com/manuals/p4sag/Content/P4SAG/security.mfa.html), [external auth triggers](https://www.perforce.com/manuals/p4sag/Content/P4SAG/triggers-external-auth.html), [SSO triggers](https://www.perforce.com/manuals/p4sag/Content/P4SAG/scripting.triggers.external_auth.sso.html)).

### 1.3 Scale numbers (what exists publicly)
- Ubisoft Montreal (case study, c. 2013): ~5 TB, 24,070,195 files, 166,642,479 revisions, 337 GB metadata, >2,000 users, three physical servers plus proxies ([IoT ONE mirror of Perforce case study](https://v2.iotone.com/case-study/ubisoft-s-game-development-enhanced-by-helix-core/c1645)).
- Warhorse Studios (2018): ~1.5 TB, 75 users, 9 TeamCity build servers ([Perforce press release](https://www.perforce.com/node/8833)).
- Perforce's own positioning: "hundreds of terabytes or even petabytes" ([Solutions Overview PDF](https://ftp.perforce.com/perforce/r21.1/doc/manuals/overview.pdf)); AWS guidance: EBS gp3 below 16 TB depot, FSx ONTAP/OpenZFS above ([AWS guidance](https://aws.amazon.com/solutions/guidance/building-perforce-helix-core-on-aws/)).
- No public per-server throughput benchmark was found; treat anything beyond the above as **unverified**.

### 1.4 Pricing pain points
- Free tier: 5 users / 20 workspaces, all features (was 20 users earlier); crossing the limit without a license caps the server at 1,000 files ([free version](https://perforce.com/products/helix-core/free-version-control), [what you get for free](https://www.perforce.com/node/323), [Swarm license note](https://perforce.com/manuals/swarm/Content/Swarm/admin.license.html), [ITCentralStation review of the 20->5 change](https://www.itcentralstation.com/products/helix-core-pricing)).
- Paid: per-user-per-year, quote only; reviewers describe cost as hard to discover and high ([PeerSpot](https://origin.peerspot.com/vendors/perforce), [TrustRadius](https://trustradius.com/products/perforce/reviews)). Helix Core Cloud is a managed tier for teams <= 50 ([Perforce blog](https://www.perforce.com/blog/vcs/helix-core-cloud-managed-hosted-version-control)); a per-seat monthly figure of $39 was reported in a search snippet but the exact page was not confirmed (**unverified**).
- Setup/admin complexity is the recurring non-price complaint ([Capterra reviews](https://www.capterra.ca/software/174325/helix-core)).

### 1.5 Security record (why "secure by default" matters)
- CVE-2023-45849 and three siblings (found by Microsoft): unauthenticated RCE as LocalSystem, CVSS 9.8, fixed in 2023.1/2513900 ([CCB Belgium advisory](https://ccb.belgium.be/advisories/warning-critical-vulnerability-perforce-helix-core-server)).
- CVE-2026-6043 (published 2026-04-24): *insecure default configuration* in P4 Server < 2026.1: when exposed to untrusted networks, unauthenticated attackers can create arbitrary users, enumerate users, log in to password-less accounts, and read depot content via the built-in `remote` user; 2026.1 enforces secure-by-default on install and upgrade; interim fix is manual hardening of security configurables ([NVD](https://nvd.nist.gov/vuln/detail/cve-2026-6043), [GCVE](https://db.gcve.eu/vuln/CVE-2026-6043), [Tenable](https://www.tenable.com/cve/CVE-2026-6043)). Lesson for us: an operations *service* must ship with no anonymous principals, no password-less accounts, and no "remote" pseudo-user; the first-run state must already be locked (P4's "everyone is super until first `p4 protect`" is the anti-pattern).

### 1.6 Adopt / avoid
- Adopt: path x principal x source-network protections with explicit deny; per-path lock policy declared by pattern (typemap-like) applied at add time; per-file retention cap (`+Sn` analogue) for working-versions; obliterate with mandatory dry run + audit; proxy/edge roles as *the same binary in different modes* fed by one replication stream.
- Avoid: single global ordered ACL table edited by hand (does not compose across teams; P4 added `owner` to patch this); "everyone is super" bootstrap; lock state that is a file *type* (so a lock policy change requires re-opening every file).

---

## 2. Unity Version Control (Plastic SCM)

### 2.1 Gluon (artist client, partial workspace)
Gluon is "for people working on a single branch with files that can't be merged"; typical user is an artist who wants to lock files so nobody edits them concurrently ([Gluon](https://docs.unity.com/en-us/unity-version-control/gluon/gluon)). The workspace is *partial*: in Explore -> Configure you tick files/folders (search by name or `*.psd` pattern) and Apply loads only those; folders are "manual mode" (subset) or "automatic mode" (whole folder); you can unload later, except items outside Gluon's control (private/ignored). CLI: `cm partial configure +/path`. Previews skip files > 1 MiB unless `MaxPreviewFileSize` in `client.conf` is raised ([configure workspace](https://docs.unity.com/en-us/unity-version-control/gluon/configure-workspace), [cm partial configure](https://docs.unity.com/ko-kr/unity-version-control/uvcs-cli/partial-configure)). Check-in releases the lock ([check in changes](https://docs.unity.com/unity-version-control/gluon/check-in-changes)). Known friction: Gluon and full workspaces are not interchangeable without converting ([Unity Discussions thread](https://discussions.unity.com/t/unity-version-control-performance-is-unacceptably-bad/1731781)); changing load state of >5,000 files was reported to "severely harm" performance on Windows ([Plastic forum, old](https://forum.plasticscm.com/topic/20563-selectingdeselecting-many-directories-with-gluon/)).

### 2.2 Smart Locks (branch-aware exclusive checkout)
Lock rules are configured centrally (Unity Dashboard -> Version Control Settings) by *file extension*; a matching file is locked on checkout even if the user chose plain Checkout. The lock has a *destination branch* (default `/main`). Checking in on a non-destination branch moves the lock to **Retained**: anyone may check out the *latest revision* of a retained file, and the lock fully clears only when the change is merged to the destination branch (or an admin releases it from the Dashboard "File locks" tab, "only as a last resort as it could enable merge conflicts") ([Smart Locks](https://docs.unity.com/en-us/unity-version-control/smart-locks), [Use Smart Locks](https://docs.unity.com/en-us/unity-version-control/use-smart-locks)). This is the most thought-through lock semantics among incumbents: it answers "what does a lock mean when there are branches?"

### 2.3 Branch-per-task, merge, mergebot
Unity's recommended workflow is one branch per issue-tracker task, justified by the engine "easily handling thousands of branches"; merges are done from Dashboard or GUI and checked in; automation through *mergebot* ([task-branch workflow](https://unity.com/how-to/devops-task-branch-workflow), [merge branches](https://docs.unity.com/en-us/unity-version-control/workflow/merge-branch)).

### 2.4 UnityYAMLMerge (semantic merge of scenes/prefabs)
Ships with the editor (`Editor/Data/Tools/UnityYAMLMerge.exe`, `Unity.app/Contents/Tools/UnityYAMLMerge`); enabled in Project Settings -> Editor once a third-party VCS (Perforce, Plastic) is selected; modes Off / Premerge (clean merges accepted, unclean ones written out as premerged base/theirs/mine for the fallback tool). Hooked into git/hg/svn/SourceTree as `merge -p $BASE $REMOTE $LOCAL $MERGED` for `.unity` and `.prefab` ([Smart Merge manual, 2019.4 mirror](https://docs.unity3d.com/ru/current/Manual/SmartMerge.html), [older 5.3 mirror](https://docs.unity3d.com/es/530/Manual/SmartMerge.html)). The important architectural point: the VCS does not merge; it delegates to a *format-aware external tool chosen by extension*, with a defined fallback.

### 2.5 Xlinks (cross-repo pointers)
An Xlink is a directory entry that points to a directory in another repository *at a specific changeset*; writable or read-only; can nest; recommended because "one repository per project" lets you set view permissions per repository ([Xlinks guide](https://docs.plasticscm.com/xlinks/plastic-scm-version-control-xlinks-guide), [2011 blog](https://blog.plasticscm.com/2011/12/how-to-link-repositories-using-xlinks.html)). Forum-reported limits: two xlinks to the same target repo are not allowed; GUI could not delete an xlink in v9 ([forum](https://forum.plasticscm.com/topic/22654-deleting-and-creating-xlinks/)).

### 2.6 Cloud, edge/proxy, on-prem
Cloud is a hosted multi-tenant server; the on-prem server runs on licence; the *proxy* (`plasticd proxy`, config `plasticcached.network.conf`, default port 8085, cache cap `MaxCacheSizeInGb`) caches cloud or on-prem data at a site ([use proxy server](https://docs.unity.com/en-us/unity-version-control/uvcs-on-prem/configure/use-proxy-server), [on-prem overview](https://docs.unity.com/en-us/unity-version-control/uvcs-on-prem/overview)). The Cloud Edition installer bundles a single-user local server; cloud repos can be *replicated/synced* to a local server ("sync view") and vice versa, but a multi-user local server needs an Enterprise licence ([forum: enterprise + private cloud](https://forum.plasticscm.com/topic/22502-enterprise-at-work-and-private-cloud-with-local-server-how/), [forum: local replication or proxy with cloud](https://forum.plasticscm.com/topic/20255-local-network-replication-or-proxy-server-with-cloud/)). No official "Cloud Edge" product page was found (**unverified** as a current product name).

### 2.7 Security model
Every object (repo, branch, item/path) has an ACL; inheritance top-down; `cm acl` sets `-allowed/-denied/-overrideallowed/-overridedenied`; secured paths `path:/src/foo.c` carry `ci, change, add, move, rm, read` and are checked on check-in and optionally on update, so a directory can be *kept out of a workspace* ([cm acl](https://docs.unity.com/en-us/unity-version-control/uvcs-cli/acl), [security guide](https://docs.unity.com/unity-version-control/security)).

### 2.8 Adopt / avoid
- Adopt: partial workspace as a first-class, server-known selection (not a client-side filter); lock rules by extension *with a destination branch and a Retained state*; extension -> merge-driver registry with premerge/fallback; cross-repo pinned pointers (xlink) with per-repo permissions; path ACL enforced on *update* (read) as well as check-in.
- Avoid: two incompatible workspace formats (Gluon vs full) requiring conversion; lock release that silently re-enables conflicts (admin force-release should leave an audit trail and warn the next checker-out).

---

## 3. Git LFS and the Git-adjacent tools

### 3.1 Git LFS pointer, batch API, transfer adapters
- **Pointer file** (spec v1): UTF-8 text, `key value\n` lines sorted alphabetically with `version https://git-lfs.github.com/spec/v1` first, then `oid sha256:<hex>`, `size <bytes>`; parsers compare version by plain string; executable bit must match the replaced file ([spec.md](https://github.com/git-lfs/git-lfs/blob/release-2.4/docs/spec.md)).
- **Batch API**: `POST <lfs>/objects/batch` with `Accept/Content-Type: application/vnd.git-lfs+json`; request `{operation: download|upload, transfers: [...], ref: {name}, objects: [{oid,size}], hash_algo}`; response `{transfer, objects: [{oid,size,authenticated, actions: {download|upload|verify: {href, header, expires_in|expires_at}}, error:{code,message}}]}`; per-object errors 404/409 (hash algo)/410/422; HTTP 401 with `LFS-Authenticate`, 403 read-only, 404 no repo, 422, optional 406/413/429/501/507/509, retryable 5xx; if the server already has the object it omits `actions` ([batch.md](https://github.com/git-lfs/git-lfs/blob/main/docs/api/batch.md)). This is the closest existing standard to Synergos' "custody service issues signed PUT/GET URLs" and is worth matching on the wire for tool compatibility (`href` + `header` + `expires_in` + `verify`).
- **Transfer adapters**: `basic` is the only standard one; `tus` (upload only) and `custom` are experimental ([API README](https://github.com/github/git-lfs/blob/master/docs/api/README.md)). **Custom transfer agents** are subprocesses configured under `lfs.customtransfer.<name>.{path,args,concurrent,direction}`, speaking one-JSON-per-line over stdin/stdout: `init` -> serial `upload`/`download` requests (`oid,size,path,action{href,header}`) with `progress` and `complete` events -> `terminate`; git-lfs verifies the SHA-256 of downloaded files itself; agents do not handle `verify`; `lfs.standalonetransferagent` runs without the batch API (actions are `null`) ([custom-transfers.md](https://github.com/git-lfs/git-lfs/blob/main/docs/custom-transfers.md)). **Design hook**: a Synergos custom-transfer agent would let any git host's LFS pointers be served by Synergos P2P/QUIC with zero client-side changes beyond config.
- **Servers**: `lfs-test-server` is explicitly "not in a production ready state" (Go, local disk, `LFS_METADB`, admin UI only if `LFS_ADMINUSER/PASS` set, optional tusd) ([lfs-test-server](https://github.com/git-lfs/lfs-test-server)). Giftless (Python) offers `basic_external` (server hands out cloud-storage URLs, never proxies bytes), `basic_streaming` (server proxies, less scalable) and a non-standard `multipart-basic` needing a special client ([Giftless transfer adapters](https://giftless.readthedocs.io/en/latest/transfer-adapters.html), [multipart spec](https://giftless.readthedocs.io/en/latest/multipart-spec.html), [PyPI](https://pypi.org/project/giftless)).

### 3.2 LFS File Locking API
`POST /locks {path, ref{name}}` -> 201 `{lock:{id,path,locked_at,owner{name}}}`, 409 with existing lock, 403 no push access; `GET /locks?path|id|cursor|limit|refspec`; `POST /locks/verify {ref,cursor,limit}` -> `{ours:[],theirs:[],next_cursor}` and a push touching a file in `theirs` is halted, 404 if locking unsupported; `POST /locks/:id/unlock {force, ref}` where non-owners need `force`. v1 is single-branch; `ref` is "for authentication only until advanced locking scenarios are defined" ([locking.md](https://github.com/git-lfs/git-lfs/blob/main/docs/api/locking.md)). Client usage is `git lfs track "*.uasset" --lockable` ([Anchorpoint on LFS locking](https://www.anchorpoint.app/blog/using-file-locking-in-git)). Weaknesses: path-only identity (renames break it), no branch semantics (contrast Unity Retained), lock enforcement only at push time.

### 3.3 Hosting limits (GitHub as the reference)
Per-file LFS caps: 2 GB Free/Pro, 4 GB Team, 5 GB Enterprise Cloud; LFS unusable with Pages and template repos ([GitHub docs source](https://github.com/github/docs/blob/main/content/repositories/working-with-files/managing-large-files/about-git-large-file-storage.md)); 1 GiB free storage and 1 GiB/month bandwidth, storage billed per *version pushed*, uploads free ([storage and bandwidth](https://docs.github.com/enterprise-cloud@latest/repositories/working-with-files/managing-large-files/about-storage-and-bandwidth-usage)). These caps are why studios leave Git for binaries.

### 3.4 Security record of LFS clients (relevant to any pointer-expanding client)
- CVE-2020-27955 (GHSA-4g4p-42wc-9f3m, 2020-11-04), CVE-2021-21237 (incomplete fix, 2021-01-13), GHSA-6rw3-3whw-jvjj (2022-04-19): executing `git.exe`/`git.bat` from the current directory on Windows, i.e. a malicious repo gets code execution ([advisory list](https://github.com/git-lfs/git-lfs/security/advisories), [OSV CVE-2024-53263](https://osv.dev/vulnerability/CVE-2024-53263)).
- CVE-2024-53263 (2025-01-14, "Clone2Leak"): CR/LF in a URL passed to `git credential` leaks credentials; fixed 3.6.1; no workaround ([Tenable](https://tenable.com/cve/CVE-2024-53263), [OSV](https://osv.dev/vulnerability/CVE-2024-53263)).
- CVE-2025-26625 (GHSA-6pvw-g552-53c5, 2025-10-17, CVSS4 8.6): `git lfs checkout/pull` follow symlinks/hardlinks and write outside the working tree (CWE-61); fixed 3.7.1; `core.symlinks=false` is only a partial mitigation ([advisory](https://github.com/git-lfs/git-lfs/security/advisories/GHSA-6pvw-g552-53c5)).
Lessons for a Synergos client that materialises blobs into a worktree: never resolve helper binaries from CWD; validate URLs byte-wise before handing them to credential helpers; check every destination path for symlink components and unlink-before-write; treat repo contents as attacker-controlled.

### 3.5 Git partial clone and sparse checkout
`git clone --filter=blob:none` omits blobs until needed; the server is a *promisor remote* (`remote.<name>.promisor`, `remote.<name>.partialclonefilter`); server must set `uploadpack.allowFilter`; `git backfill` fetches missing blobs in batches (`--sparse` restricts to the sparse-checkout) ([git-clone](https://git-scm.com/docs/git-clone/2.47.0), [git-config](https://git-scm.com/docs/git-config/2.50.0), [git-backfill](https://git-scm.com/docs/git-backfill)). Newer Git documents *large-object promisors* (a separate remote that serves large blobs, advertised through protocol v2, selected with `--filter=auto`) ([large-object-promisors](https://git-scm.com/docs/large-object-promisors), [gitprotocol-v2](https://git-scm.com/docs/gitprotocol-v2)). Sparse checkout cone mode restricts the *worktree* to directories; combined with blobless clone it bounds both disk and fetched objects ([GitLab partial clone guide](https://docs.Gitlab.com/ee/topics/git/partial_clone.html)). **Design hook**: a Synergos history node could present itself as a large-object promisor remote, which is the Git-native way to say "text in git, blobs elsewhere" without LFS pointers.

### 3.6 git-annex
Annexed files become symlinks to `.git/annex/objects/<key>`; keys come from pluggable backends (default WORM = basename+size+mtime; hash backends exist); `drop` is refused unless `annex.numcopies` (default 1) other copies are known; *special remotes* (S3, WebDAV, rsync, directory, bup, Glacier, bittorrent) hold content outside Git; location tracking can go stale, symlinks break Windows and some tools, repo format has changed v5..v10 ([git-annex man page](https://man.archlinux.org/man/git-annex.1)). Take the *numcopies* idea: a blob may be garbage-collected or dropped only when a verified count of replicas exists elsewhere.

### 3.7 Diversion
Cloud-hosted, no servers to run, work-in-progress synced to cloud in real time ([game development page](https://www.diversion.dev/game-development)). **Soft locks** by default: the service detects that someone else is editing a file and prompts the second editor; they may proceed. **Hard locks** (Studio/Enterprise plans) block the second commit and can auto-lock by extension; stated rationale is avoiding Perforce's "locker is unavailable" deadlock ([soft locks](https://docs.diversion.dev/soft-locks), [coming from Perforce](https://docs.diversion.dev/coming_from_perforce)). Unreal plugin: Revision Control -> Check In = commit; Blueprints mergeable via the engine, `.umap/.uasset/.fbx` not ([soft locks](https://docs.diversion.dev/soft-locks)). Free tier 5 users / 100 GB ([Fab listing](https://www.fab.com/listings/59d591f2-4026-4e8f-9128-9c8ef1ff9bab)). All vendor-stated.

### 3.8 Anchorpoint
A Git client with "an additional metadata system attached": artist UI ("2 buttons"), auto-configures LFS, and its own real-time lock service that bypasses LFS lock limits; Unreal/Unity plugins with Blueprint diff; EUR 20/user/month, 50% indie discount ([Git for artists](https://www.anchorpoint.app/features/git-for-artists), [file locking blog](https://www.anchorpoint.app/blog/using-file-locking-in-git), [UE forum thread](https://forums.unrealengine.com/t/anchorpoint-anchorpoint-version-control-with-git/2567882)). Pattern: keep Git as the *storage/protocol* and add a side-channel service for the things Git cannot express (locks, previews, who-is-editing).

### 3.9 Snowtrack -> Perforce P4 One
Snowtrack ("git for artists", visual history for Blender/Adobe/Unreal, renders GLTF/FBX/OBJ/HDR/EXR/PSD) was acquired by Perforce in March 2025 and relaunched as **P4 One**, a local-first artist client that still talks to a P4 server (exclusive locking, native binary support); closed beta at GDC 2025 with "global launch later in 2025"; Perforce claims up to 10x faster than Git on very large datasets (vendor) ([Perforce press release](https://www.perforce.com/press-releases/announcing-p4-one), [PocketGamer](https://www.pocketgamer.biz/perforce-acquires-snowtrack-with-closed-beta-launch-for-digital-artists), [P4 platform blog](https://www.perforce.com/blog/vcs/introducing-the-p4-platform)). GA status as of 2026-10 **unverified**.

### 3.10 Alienbrain
Late-1990s NXN product, now Southpaw (2018); centralized VCS + DAM for 2D/3D artists with Maya/3ds Max/Photoshop and (2019) Unreal Editor integration: check-out/check-in from inside the DCC; thumbnails, tagging, locking; criticised for Windows-only, licensing cost and slow development ([CGW 2019](https://www.cgw.com/Press-Center/News/2019/Alienbrain-2019-Ships.aspx), [Anchorpoint comparison](https://www.anchorpoint.app/blog/compare-alienbrain-alternatives)). Shows the long-standing demand for *previews + locks + DCC plugins* in one product.

### 3.11 Subversion / TortoiseSVN
`svn:needs-lock` makes the working-copy file read-only on checkout/update; `svn lock` makes it writable; release or defunct lock makes it read-only again; the property *reminds*, it does not enforce; cannot be set recursively on folders ([svn-needs-lock design note](https://apache.googlesource.com/subversion/+/fsx/notes/locking/svn-needs-lock-impl.txt), [TortoiseSVN locking](https://documentation.help/TortoiseSVN/tsvn-dug-locking.html), [VisualSVN lock-modify-unlock](https://visualsvn.com/support/topic/00016)). A pre-commit hook rejecting binaries without the property is the common enforcement ([Orcaware hook](https://orcaware.com/svn/wiki/Automatic_lock-modify-unlock)). The *read-only-bit-as-UI* trick is cheap and effective and should be copied by any client that materialises lockable files.

### 3.12 Helix DAM
Web DAM on top of a P4D server: links to existing streams or classic depots, auto-tags uploads, indexes audio/video, hi-res thumbnails incl. USD, shows lock state, comments, Kanban review states, Jira integration; plugins for Photoshop/Maya/Blender/Helix Sync; Azure Marketplace listing ([product brief](https://www.perforce.com/sites/default/files/pdfs/product-brief-helix-dam.pdf), [connect DAM to streams](https://perforce.com/node/1187282), [what's new 2024.2](https://www.perforce.com/sites/default/files/pdfs/whats-new-helix-dam-2024-2.pdf)). This is the layer Curare is competing with: catalog + preview + review + lock visibility over a blob store.

---

## 4. Unreal Engine's data pipeline

- **One File Per Actor (OFPA)**: each actor instance is its own external file so level edits no longer contend on the `.umap`; default with World Partition; editor-only (actors are embedded when cooked); file names are encoded, so the editor's View Changelist window exists to validate what you submit ([OFPA](https://dev.epicgames.com/documentation/en-us/unreal-engine/one-file-per-actor-in-unreal-engine)). Consequence: thousands of small binary files per level, which a third-party vendor claims Git LFS handles poorly and P4 well ([USourceControl guide](https://usourcecontrol.com/guides/one-file-per-actor-source-control), vendor claim). Our manifest/pointer model must handle **many small blobs** (batching, directory-level manifests), not just big ones.
- **Virtual Assets**: bulk payloads are stripped from `.uasset` and stored in a *backend graph* (persistent: Perforce depot with a `payload_metainfo.txt` marker; cache: DDC); the payload identifier is the **FIoHash = BLAKE3-160 (first 20 bytes of BLAKE3-256) of the uncompressed payload**; duplicate payloads in a package are stored once in the package trailer; a failed pull aborts the editor ("high risk of data corruption"); `UnrealVirtualizationTool` rehydrates for shipping outside your infra; only Perforce is supported as persistent backend, feature is Beta ([overview](https://dev.epicgames.com/documentation/en-us/unreal-engine/overview-of-virtual-assets-in-unreal-engine), [quickstart](https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-assets-quickstart-in-unreal-engine), [backend graphs](https://dev.epicgames.com/documentation/en-us/unreal-engine/backend-graphs-for-virtual-assets-in-unreal-engine), [FIoHash API](https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Core/FIoHash), [FPushRequest](https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Core/FPushRequest/__ctor), [UE 5.1 tech blog](https://www.unrealengine.com/tech-blog/virtual-assets-in-unreal-engine-5-1-smaller-faster-data-syncs)). **This is a direct fit for Synergos**: UE already content-addresses payloads with BLAKE3; a Synergos backend for the VA graph is a plausible product.
- **Zen storage server**: default local DDC since UE 5.4 and usable as shared DDC; *unauthenticated*, intended for LAN/VPN only, server-managed GC ([Zen shared DDC](https://dev.epicgames.com/documentation/en-us/unreal-engine/set-up-zen-storage-server-as-shared-ddc-for-unreal-engine), [DDC](https://dev.epicgames.com/documentation/unreal-engine/using-derived-data-cache-in-unreal-engine)).
- **Unreal Cloud DDC**: distributed CAS for untrusted networks; payloads content-addressed by hash; Scylla for metadata, S3 for payloads, per-node NVMe hot cache (~7 TB/node, ~50 TB/region for two months of builds per an Epic talk), multi-region replication; production only on AWS; Docker-compose dev setups disable auth and Epic says to hook up an **OIDC provider** before deploying; Epic's OIDC token tool mints/refreshes tokens for the editor ([Cloud DDC setup](https://dev.epicgames.com/documentation/en-us/unreal-engine/how-to-set-up-a-cloud-type-derived-data-cache-for-unreal-engine), [OIDC tokens](https://dev.epicgames.com/documentation/unreal-engine/creating-oidc-tokens-for-unreal-engine), [The New Stack on Epic's ScyllaDB talk](https://thenewstack.io/how-epic-games-revs-up-unreal-engine-cook-time-for-devs/), [Epic Japan deck](https://docswell.com/s/EpicGamesJapan/ZV172L-Improving%20Development%20Iterations)).
- **Horde**: Epic's CI/CD and agent farm; runs BuildGraph on agents with synced P4 workspaces; requires a P4 *stream* (classic branches unsupported); can read its own config from P4 ([Horde build automation](https://dev.epicgames.com/documentation/en-us/unreal-engine/horde-build-automation-for-unreal-engine), [tutorial](https://dev.epicgames.com/documentation/en-us/unreal-engine/horde-build-automation-tutorial-for-unreal-engine)).

Adopt: BLAKE3-based payload identity (already aligned), backend-graph (tiered cache -> persistent) semantics, OIDC for anything reachable off-LAN. Avoid: Zen's "no auth, trust the LAN" posture for anything we operate as a service.

---

## 5. AI/ML data versioning

### 5.1 DVC
`.dvc/cache` is content-addressable (MD5 by default, 32-char hash names); `dvc.lock` records dependency/output hashes per pipeline stage and is committed to Git; the **run cache** (`.dvc/cache/runs`) logs a signature per stage run and restores outputs instantly on identical conditions, disabled per stage with `cache: false`, pushed/pulled with the normal cache ([run cache](https://dvc.org/doc/user-guide/pipelines/run-cache), [internal files](https://doc.dvc.org/user-guide/project-structure/internal-files), [DVC 1.0 thread](https://discuss.dvc.org/t/dvc-1-0-release/412)). In Nov 2025 lakeFS acquired the DVC project; DVC continues as an independent OSS tool for "smaller datasets" ([lakeFS blog](https://lakefs.io/blog/lakefs-acquires-dvc/)). Limits: MD5 identity, whole-file granularity, Git-coupled metadata, no server-side policy.

### 5.2 lakeFS
Git semantics over object storage: a branch is a pointer to a commit plus uncommitted changes; branching is zero-copy; commits are sets of pointers to immutable physical objects, so a changed object yields new pointers while old commits keep old objects ([model](https://docs.lakefs.io/understand/model/)). Internals ("Graveler"): each commit = a set of contiguous non-overlapping **SSTables (PebbleDB format)** covering the whole keyspace; a *metarange* indexes *ranges* ("a B+tree of height 2"); files are immutable and cached; unchanged ranges are reused across commits; stored under `_lakefs/` in the bucket ([versioning internals](https://docs.lakefs.io/understand/how/versioning-internals/), [Concrete Graveler](https://lakefs.io/blog/concrete-graveler-committing-data-to-pebbledb-sstables/)). **lakeFS Mount** exposes any path at any branch/commit/tag as a local filesystem with lazy or eager fetch; when mounted inside a Git repo, Git records which data version was used ([lakeFS Mount](https://lakefs.io/lp-lakefs-mount/)). Security: IAM-style RBAC (users/groups/policies with allow/deny, deny wins) only on Cloud/Enterprise; presigned URLs, short-lived tokens, SSO, AWS IAM login ([RBAC](https://docs.lakefs.io/security/rbac/), [security](https://docs.lakefs.io/security/)). **Adopt**: the metarange/range immutable-index design is the right shape for a Curare "snapshot of a dataset" that must diff cheaply and share unchanged subtrees.

### 5.3 Hugging Face Xet (ex-XetHub)
HF acquired XetHub (Aug 2024) "to replace Git LFS on the Hub"; LFS remains supported through a bridge for legacy clients ([Hub Xet docs](https://huggingface.co/docs/hub/xet)). Mechanism: content-defined chunking with a rolling hash, target ~64 KB chunks (spec: 8 KB-128 KB range, GearHash); new chunks are grouped into ~64 MB *xorbs* (blocks) that are content-addressed in a CAS; uploads push xorbs first, then *shards* that reference them; dedup lookup is layered: current session -> local metadata cache -> a *global* query against all of Xet storage for a subset of chunks; server-side `hf buckets cp` copies hashes only; recommended per-file limit 200 GB; a 20 GB file is ~312,500 chunks ([dedup (hub docs source)](https://github.com/huggingface/hub-docs/blob/main/docs/hub/xet/deduplication.md), [Xet spec](https://huggingface.co/docs/xet/deduplication.md), [upload protocol](https://huggingface.co/docs/xet/upload-protocol), [xet-core](https://github.com/huggingface/xet-core)). Results reported by HF: ~50% storage/transfer improvement vs LFS across three iterative workflows; CORD-19 dataset 8.9 GB (LFS) vs 3.52 GB (Xet) ([From Files to Chunks](https://huggingface.co/blog/from-files-to-chunks), [From Chunks to Blocks](https://huggingface.co/blog/from-chunks-to-blocks), [CIDR 2023 slides](https://www.cidrdb.org/cidr2023/slides/p43-low-slides.pdf)). Migration reality: "migrated" repos could still contain LFS files pending a second sweep ([Hub discussion](https://huggingface.co/spaces/jsulz/ready-xet-go/discussions/1)). Client telemetry is sent to CAS by default, opt-out via `HF_XET_TELEMETRY_ENABLED=0` ([xet-core README](https://github.com/huggingface/xet-core)).
**Relevance to the 2026-08-16 "no CDC" decision**: Xet is the strongest evidence that CDC pays for *iteratively edited large files* (model checkpoints, datasets) and is neutral-to-useless for already-compressed game media. Global dedup across tenants is also a **confidentiality side channel** (chunk-existence oracle) unless dedup is scoped per tenant/key; HF only says cross-repo sharing is "secure" without detail. If CDC is ever revisited, it should be opt-in per asset class and tenant-scoped.

### 5.4 Pachyderm
Repos, branches (alias of a commit), immutable commits, and **provenance**: a derived commit records the source commits it was computed from; every step of a pipeline DAG shares a *global ID* so related commits across repos are found together ([intro to data versioning](https://docs.pachyderm.com/2.8.x/learn/intro-data-versioning), [provenance glossary](https://docs.pachyderm.com/2.8.x/learn/glossary/provenance)). Storage details for 2.x (chunking, file sets) were not retrievable from a primary source in this session (**unverified**); 1.x used hashtrees and *local* dedup against a datum's previous outputs ([Pachyderm 1.8 blog](https://pachyderm.com/blog/pachyderm-1-8-performance-improvements)). Acquired by HPE. Take the *provenance edge + global run ID* for Curare's "reproducible" version type.

### 5.5 Dolt
MySQL-compatible DB with Git semantics; tables are **Prolly trees** (B-trees whose node boundaries are content-defined, so equal data => equal nodes), roots stored in a commit graph; diff/merge are structural rather than full scans; Dolt 2.0 (2026) added automatic GC/compression and large/vector types; DoltLite = SQLite with a prolly-tree engine ([storage engine docs](https://docs.dolthub.com/architecture/storage-engine), [DoltHub blog](https://dolthub.com/blog/2024-02-29-storage-engine), [LWN](https://lwn.net/Articles/1068864/), [InfoQ](https://infoq.com/news/2026/07/dolt-version-control)). Relevance: the *manifest* in our design (millions of path -> hash entries) should be a prolly/Merkle structure, not a flat file, so manifest diffs and partial sync are O(changed subtrees).

### 5.6 Weights & Biases Artifacts
Logging an artifact checksums each file; a new version is created only if the digest differs (same digest = no-op); storage is charged only for changed files (example: v0 3 MB, v1 adds 3 MB -> v1 logically 6 MB, physically 3 MB; deleting v0 shifts cost to v1); reference artifacts track S3/GCS/Azure objects by checksum without copying (default cap 10,000,000 objects, `max_objects`); lineage (produced-by / used-by DAG) is automatic ([artifact storage](https://docs.wandb.ai/models/support/artifact_storage_version), [Artifact API](https://docs.wandb.ai/models/ref/python/experiments/artifact), [W&B overview report](https://wandb.ai/wandb/customers/reports/W-B-Artifacts-Overview--VmlldzoxMTE2NjM2Ng)). Whether blobs are content-addressed internally is not stated officially (**unverified**).

### 5.7 MLflow artifacts
The tracking server either *proxies* artifact I/O (`--artifacts-destination s3://...`; only the server holds credentials, and "anyone who can reach the server can act with the server's role") or hands clients direct URIs (`--no-serve-artifacts --default-artifact-root`), which pushes credentials to every client; artifact location is frozen per experiment at creation ([tracking server](https://mlflow.org/docs/latest/ml/tracking/server), [self-hosting architecture](https://mlflow.org/docs/latest/self-hosting/architecture/tracking-server/)). Our custody service's short-lived HMAC-signed URLs are the middle path MLflow lacks; keep it.

### 5.8 Oxen.ai
Rust, git-like CLI/HTTP API, blobs + Merkle-tree metadata, per-file-type metadata cached in the tree, tabular files indexed server-side (DuckDB per a third party); claims millions of files / terabytes and "hundreds of thousands of files in seconds" (unquantified vendor claims); block-level dedup is an experiment directory, not a feature; `oxen-server` does not run natively on Windows; remote "workspaces" let users add files without a local clone ([Oxen repo](https://github.com/Oxen-AI/Oxen), [performance page](https://docs.oxen.ai/features/performance), [versioning](https://docs.oxen.ai/getting-started/versioning)).

### 5.9 Delta Lake / Apache Iceberg snapshots
- **Iceberg**: a snapshot = set of live data files; per-snapshot manifest list -> immutable manifests (reused across snapshots); every table change writes a new metadata file and **atomically swaps** the pointer (optimistic concurrency: retry on a stale base); sequence numbers order data/delete files; refs = branches/tags in the metadata `refs` map with `min-snapshots-to-keep` (default from `history.expire.min-snapshots-to-keep`=1), `max-snapshot-age-ms`, `max-ref-age-ms` (tags keep exactly one snapshot; `main` never expires); expiration marks via refs first, then deletes files only if `gc.enabled` ([Iceberg spec](https://raw.githubusercontent.com/apache/iceberg/main/format/spec.md), [spec page](https://iceberg.apache.org/spec/), [Dremio on expiration](https://www.dremio.com/blog/apache-iceberg-snapshot-expiration/), [snapshot references](https://iceberglakehouse.com/iceberg/iceberg-snapshot-references/)).
- **Delta**: `_delta_log/<zero-padded version>.json` commits + Parquet/JSON checkpoints + `_last_checkpoint`; versions are contiguous integers; `remove` actions are tombstones with `deletionTimestamp`, kept until retention (default 7 days) then VACUUM deletes files; catalog-managed tables ratify versions with PUT-if-absent ([Delta PROTOCOL.md](https://raw.githubusercontent.com/delta-io/delta/master/PROTOCOL.md)); `delta.deletedFileRetentionDuration` default 7 days, `delta.logRetentionDuration` default 30 days, time travel only works within both windows ([Databricks VACUUM](https://docs.databricks.com/aws/sql/language-manual/delta-vacuum)).
- Take: *named refs with per-ref retention policies* and *tombstone + grace period + GC* as the formal model for "working" vs "release" versions in Curare, and PUT-if-absent / compare-and-swap on a single metadata pointer as the commit protocol for history nodes.

### 5.10 Dataset cards and lineage
HF dataset card = `README.md` with a YAML header (license, language, size, tags, configs/splits) rendered on the Hub and used for filtering; modelled on Model Cards (Mitchell 2018) ([datasets cards](https://huggingface.co/docs/hub/en/datasets-cards)). A study of 7,433 cards found 86.0% of the top-100 downloaded datasets complete every suggested section vs 7.9% of never-downloaded ones ([ICLR 2024](https://iclr.cc/virtual/2024/poster/17463)). No HF-native provenance graph was found; lineage is a product feature in W&B and Pachyderm, not in the card format (**gap in sources**).

---

## 6. Cross-cutting lessons for Synergos + Curare

1. **Locks are policy + state + branch semantics**, not a boolean. Best-of-breed = P4's pattern-declared policy + Unity's destination-branch/Retained state + LFS's HTTP API shape + Diversion's soft-lock prompt + SVN's read-only bit as UI. None of the incumbents key locks by content hash; all key by path, which breaks on rename.
2. **Topology roles should be one binary**: proxy (cache only) / forwarding replica (read-serve, write-forward) / edge (local metadata, commit elsewhere). P4 proves this scales; its pain is that each role needs separate backup/provisioning. Synergos' history nodes + relay already resemble this; the missing piece is a *declared* role model with replication-stream semantics.
3. **Partial checkout must be server-known** (Gluon's configure, git sparse-checkout + promisor, lakeFS Mount) so the server can enforce path ACLs on *read* and so transfer can be planned.
4. **Manifests need Merkle/prolly structure** (lakeFS metarange, Dolt, Oxen, Iceberg manifests) once file counts hit OFPA scale.
5. **Every version whole-file** is the P4/LFS/W&B model and is fine *if* paired with per-file revision caps (`+Sn`), tombstone + grace + GC (Delta/Iceberg), numcopies-before-drop (git-annex), and obliterate with dry-run + audit (P4). CDC (Xet) wins only for iteratively-edited large files and introduces a cross-tenant dedup side channel.
6. **Secure-by-default is a product requirement, not hardening**: P4's CVE-2026-6043 was a *default*; Zen is unauthenticated by design; MLflow proxy mode makes the server a credential; LFS clients had three CWD-execution and one symlink-escape CVE. Design the first-run state, the client's file-materialisation path, and the credential-helper boundary explicitly.
7. **Wire compatibility is cheap leverage**: implementing the LFS batch + locking JSON and a custom-transfer agent lets any Git host or DCC plugin that already speaks LFS use Synergos; a UE Virtual Assets backend and a Git large-object promisor are the two other high-value adapters.

---

## 7. Need -> incumbent mechanism -> status in Synergos/Curare

Status values: `unknown` (not visible from the task context), `absent`, `partial`. The team fills the last column from the code; entries marked "per context" are inferred from the task description only.

| Need | Incumbent mechanism (best reference) | Status in Synergos/Curare |
|---|---|---|
| Exclusive checkout of unmergeable binaries | P4 `+l` via typemap; Unity Smart Locks (extension rules, destination branch, Retained); LFS `/locks` API | unknown |
| Soft "someone is editing" signal | Diversion soft locks; Anchorpoint real-time metadata | unknown |
| Lock policy declared by path/extension pattern | P4 typemap; Unity Dashboard lock rules; `git lfs track --lockable` | unknown |
| Lock semantics across branches | Unity Retained lock + destination branch | unknown |
| Admin force-unlock with audit | LFS `unlock {force}`; Unity Dashboard release | unknown |
| Path x principal x source-network ACL with explicit deny | P4 protections table; Unity `cm acl` secured paths (checked on update) | unknown (Cernere policy; granularity unverified) |
| Secure first-run state (no anonymous / default principals) | P4 2026.1 secure-by-default (after CVE-2026-6043) | unknown |
| MFA / SSO / OIDC for off-LAN services | P4 2fa triggers; Unreal Cloud DDC OIDC; lakeFS SSO | unknown |
| Partial / sparse workspace known to server | Gluon configure; git sparse-checkout + promisor; lakeFS Mount | unknown |
| Site cache / proxy | P4 Proxy (p4p); Unity proxy (8085, MaxCacheSizeInGb) | partial per context (Cloudflare relay exists; cache role unverified) |
| Regional read replica with write forwarding | P4 forwarding replica | unknown |
| Edge metadata server with central commit | P4 commit/edge | unknown |
| Content-addressed blob identity | UE FIoHash BLAKE3-160; LFS sha256; Xet xorb hash | partial per context (BLAKE3 whole-file) |
| Signed, expiring transfer URLs with verify step | LFS batch `actions{href,header,expires_in}` + `verify`; lakeFS presigned; MLflow proxy mode | partial per context (HMAC PUT/GET + SHA-256 receipts) |
| Standard client adapter (LFS custom transfer / promisor / UE VA backend) | LFS custom-transfers.md; git large-object-promisors; UE backend graph | absent per context |
| Many-small-files efficiency (OFPA) | P4 (vendor claim); Merkle manifests (lakeFS/Dolt/Oxen) | unknown |
| Per-file revision retention cap | P4 `+Sn` | unknown |
| Tombstone + grace period + GC | Delta `deletionTimestamp` + VACUUM; Iceberg expire snapshots | unknown |
| Refs with per-ref retention policy | Iceberg `min-snapshots-to-keep` / `max-snapshot-age-ms` / `max-ref-age-ms` | unknown |
| Replica-count-before-drop | git-annex `numcopies` | unknown |
| Obliterate (legal removal) with dry run + audit | P4 `obliterate` (`-y`) | unknown |
| Atomic commit protocol (CAS on metadata pointer) | Iceberg atomic swap; Delta PUT-if-absent | unknown |
| Format-aware merge driver registry with fallback | UnityYAMLMerge premerge; UE Blueprint merge | unknown |
| Cross-repo pinned pointers | Plastic xlinks; git submodules | unknown |
| Branch-per-task + mergebot | Unity task branches | unknown |
| Zero-copy branching over object storage | lakeFS branch = pointer | unknown |
| Run/pipeline cache and reproducible stage signatures | DVC run cache + dvc.lock; Pachyderm provenance + global ID | partial per context ("reproducible" version type exists; mechanism unverified) |
| Lineage / provenance graph | W&B artifact DAG; Pachyderm provenance | unknown |
| Dataset/model documentation (cards) | HF dataset cards (YAML header) | unknown |
| Dedup across versions of iteratively edited large files | Xet CDC (64 KB chunks, 64 MB xorbs) | absent (decided 2026-08-16) |
| Tenant-scoped dedup (no cross-tenant chunk oracle) | not solved publicly by Xet docs | absent (n/a while no CDC) |
| Previews / thumbnails / DCC plugins / review board | Helix DAM; Alienbrain; P4 One; Anchorpoint | unknown (Curare catalog scope) |
| Client hardening: no CWD binary lookup, URL control-char filtering, symlink-safe materialisation | Git LFS CVE-2020-27955 / 2024-53263 / 2025-26625 fixes | unknown |
| Hosting quotas (per-file size, bandwidth accounting) | GitHub LFS 2/4/5 GB, 1 GiB quotas | unknown |

---

## Gaps in sources (could not be confirmed)
- Perforce per-seat list price and any current per-server throughput benchmark.
- Current product name/existence of a Plastic "Cloud Edge" server.
- Pachyderm 2.x storage-layer chunking details.
- Unreal Cloud DDC README (repo returned 404 without an Epic-linked account) and namespace-level access control.
- Whether W&B stores blobs content-addressed internally.
- Hugging Face "From Files to Chunks" full text (host blocked; numbers taken from search excerpts of the HF blog).
- P4 One general-availability status after the 2025 closed beta.

---

## Sources
Perforce
- https://www.perforce.com/manuals/p4sag/Content/P4SAG/Commit-edge-overview.html
- https://perforce.com/manuals/v23.1/p4sag/Content/P4SAG/deployment-architecture.html
- https://www.perforce.com/manuals/p4sag/Content/P4SAG/forwarding-replica.html
- https://www.perforce.com/manuals/p4sag/Content/P4SAG/chapter.proxy.html
- https://ftp.perforce.com/perforce/r18.2/doc/manuals/p4dist/Content/P4Dist/proxy.performance.preload_cache.html
- https://www.perforce.com/manuals/p4dist/Content/P4Dist/chapter.proxy.html
- https://www.perforce.com/perforce/r16.1/manuals/cmdref/p4_protect.html
- https://perforce.com/manuals/cmdref/Content/CmdRef/p4_protect.html
- https://www.perforce.com/manuals/v15.1/p4sag/chapter.protections.html
- https://www.perforce.com/manuals/v20.2/p4sag/Content/P4SAG/protections.set.html
- https://perforce.com/manuals/cmdref/Content/CmdRef/p4_typemap.html
- https://www.perforce.com/manuals/v16.1/p4sag/chapter.configuring.html
- https://www.perforce.com/manuals/cmdref/Content/CmdRef/file.types.html
- https://www.perforce.com/manuals/v17.1/cmdref/Content/P4Guide/filetypes.modifiers.html
- https://www.perforce.com/manuals/v17.1/cmdref/Content/P4Guide/filetypes.storage.html
- https://perforce.com/manuals/p4sag/Content/P4SAG/performance.prevention.compression.html
- https://perforce.com/perforce/doc.092/manuals/cmdref/obliterate.html
- https://www.perforce.com/manuals/v23.2/p4v/Content/P4V/streams.about.html
- https://ftp.perforce.com/perforce/r19.1/doc/manuals/cmdref/Content/CmdRef/p4_stream.html
- https://perforce.com/manuals/p4sag/Content/P4SAG/security.mfa.html
- https://www.perforce.com/manuals/p4sag/Content/P4SAG/triggers-external-auth.html
- https://www.perforce.com/manuals/p4sag/Content/P4SAG/scripting.triggers.external_auth.sso.html
- https://nvd.nist.gov/vuln/detail/cve-2026-6043
- https://db.gcve.eu/vuln/CVE-2026-6043
- https://www.tenable.com/cve/CVE-2026-6043
- https://ccb.belgium.be/advisories/warning-critical-vulnerability-perforce-helix-core-server
- https://perforce.com/products/helix-core/free-version-control
- https://www.perforce.com/node/323
- https://perforce.com/manuals/swarm/Content/Swarm/admin.license.html
- https://www.itcentralstation.com/products/helix-core-pricing
- https://origin.peerspot.com/vendors/perforce
- https://trustradius.com/products/perforce/reviews
- https://www.capterra.ca/software/174325/helix-core
- https://www.perforce.com/blog/vcs/helix-core-cloud-managed-hosted-version-control
- https://www.perforce.com/products/helix-core
- https://v2.iotone.com/case-study/ubisoft-s-game-development-enhanced-by-helix-core/c1645
- https://www.perforce.com/node/8833
- https://ftp.perforce.com/perforce/r21.1/doc/manuals/overview.pdf
- https://aws.amazon.com/solutions/guidance/building-perforce-helix-core-on-aws/
- https://blog.assembla.com/the-top-5-pros-and-cons-of-perforce-helix-core-edge-servers-vs-forwarding-replicas
- https://www.perforce.com/sites/default/files/pdfs/product-brief-helix-dam.pdf
- https://perforce.com/node/1187282
- https://www.perforce.com/sites/default/files/pdfs/whats-new-helix-dam-2024-2.pdf
- https://www.perforce.com/press-releases/announcing-p4-one
- https://www.pocketgamer.biz/perforce-acquires-snowtrack-with-closed-beta-launch-for-digital-artists
- https://www.perforce.com/blog/vcs/introducing-the-p4-platform

Unity Version Control / Plastic SCM
- https://docs.unity.com/en-us/unity-version-control/gluon/gluon
- https://docs.unity.com/en-us/unity-version-control/gluon/configure-workspace
- https://docs.unity.com/ko-kr/unity-version-control/uvcs-cli/partial-configure
- https://docs.unity.com/unity-version-control/gluon/check-in-changes
- https://forum.plasticscm.com/topic/20563-selectingdeselecting-many-directories-with-gluon/
- https://discussions.unity.com/t/unity-version-control-performance-is-unacceptably-bad/1731781
- https://docs.unity.com/en-us/unity-version-control/smart-locks
- https://docs.unity.com/en-us/unity-version-control/use-smart-locks
- https://unity.com/how-to/devops-task-branch-workflow
- https://docs.unity.com/en-us/unity-version-control/workflow/merge-branch
- https://docs.unity3d.com/ru/current/Manual/SmartMerge.html
- https://docs.unity3d.com/es/530/Manual/SmartMerge.html
- https://docs.plasticscm.com/xlinks/plastic-scm-version-control-xlinks-guide
- https://blog.plasticscm.com/2011/12/how-to-link-repositories-using-xlinks.html
- https://forum.plasticscm.com/topic/22654-deleting-and-creating-xlinks/
- https://docs.unity.com/en-us/unity-version-control/uvcs-on-prem/configure/use-proxy-server
- https://docs.unity.com/en-us/unity-version-control/uvcs-on-prem/overview
- https://forum.plasticscm.com/topic/22502-enterprise-at-work-and-private-cloud-with-local-server-how/
- https://forum.plasticscm.com/topic/20255-local-network-replication-or-proxy-server-with-cloud/
- https://docs.unity.com/en-us/unity-version-control/uvcs-cli/acl
- https://docs.unity.com/unity-version-control/security

Git LFS, Git, and Git-adjacent tools
- https://github.com/git-lfs/git-lfs/blob/release-2.4/docs/spec.md
- https://github.com/git-lfs/git-lfs/blob/main/docs/api/batch.md
- https://github.com/git-lfs/git-lfs/blob/main/docs/api/locking.md
- https://github.com/git-lfs/git-lfs/blob/main/docs/custom-transfers.md
- https://github.com/github/git-lfs/blob/master/docs/api/README.md
- https://github.com/git-lfs/git-lfs/security/advisories
- https://github.com/git-lfs/git-lfs/security/advisories/GHSA-6pvw-g552-53c5
- https://osv.dev/vulnerability/CVE-2024-53263
- https://tenable.com/cve/CVE-2024-53263
- https://github.com/github/docs/blob/main/content/repositories/working-with-files/managing-large-files/about-git-large-file-storage.md
- https://docs.github.com/enterprise-cloud@latest/repositories/working-with-files/managing-large-files/about-storage-and-bandwidth-usage
- https://github.com/git-lfs/lfs-test-server
- https://giftless.readthedocs.io/en/latest/transfer-adapters.html
- https://giftless.readthedocs.io/en/latest/multipart-spec.html
- https://pypi.org/project/giftless
- https://git-scm.com/docs/git-clone/2.47.0
- https://git-scm.com/docs/git-config/2.50.0
- https://git-scm.com/docs/git-backfill
- https://git-scm.com/docs/large-object-promisors
- https://git-scm.com/docs/gitprotocol-v2
- https://docs.Gitlab.com/ee/topics/git/partial_clone.html
- https://man.archlinux.org/man/git-annex.1
- https://docs.diversion.dev/soft-locks
- https://docs.diversion.dev/coming_from_perforce
- https://www.diversion.dev/game-development
- https://www.fab.com/listings/59d591f2-4026-4e8f-9128-9c8ef1ff9bab
- https://www.anchorpoint.app/features/git-for-artists
- https://www.anchorpoint.app/blog/using-file-locking-in-git
- https://forums.unrealengine.com/t/anchorpoint-anchorpoint-version-control-with-git/2567882
- https://www.cgw.com/Press-Center/News/2019/Alienbrain-2019-Ships.aspx
- https://www.anchorpoint.app/blog/compare-alienbrain-alternatives
- https://apache.googlesource.com/subversion/+/fsx/notes/locking/svn-needs-lock-impl.txt
- https://documentation.help/TortoiseSVN/tsvn-dug-locking.html
- https://visualsvn.com/support/topic/00016
- https://orcaware.com/svn/wiki/Automatic_lock-modify-unlock

Unreal Engine
- https://dev.epicgames.com/documentation/en-us/unreal-engine/one-file-per-actor-in-unreal-engine
- https://usourcecontrol.com/guides/one-file-per-actor-source-control
- https://dev.epicgames.com/documentation/en-us/unreal-engine/overview-of-virtual-assets-in-unreal-engine
- https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-assets-quickstart-in-unreal-engine
- https://dev.epicgames.com/documentation/en-us/unreal-engine/backend-graphs-for-virtual-assets-in-unreal-engine
- https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Core/FIoHash
- https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Core/FPushRequest/__ctor
- https://www.unrealengine.com/tech-blog/virtual-assets-in-unreal-engine-5-1-smaller-faster-data-syncs
- https://dev.epicgames.com/documentation/en-us/unreal-engine/set-up-zen-storage-server-as-shared-ddc-for-unreal-engine
- https://dev.epicgames.com/documentation/unreal-engine/using-derived-data-cache-in-unreal-engine
- https://dev.epicgames.com/documentation/en-us/unreal-engine/how-to-set-up-a-cloud-type-derived-data-cache-for-unreal-engine
- https://dev.epicgames.com/documentation/unreal-engine/creating-oidc-tokens-for-unreal-engine
- https://thenewstack.io/how-epic-games-revs-up-unreal-engine-cook-time-for-devs/
- https://docswell.com/s/EpicGamesJapan/ZV172L-Improving%20Development%20Iterations
- https://dev.epicgames.com/documentation/en-us/unreal-engine/horde-build-automation-for-unreal-engine
- https://dev.epicgames.com/documentation/en-us/unreal-engine/horde-build-automation-tutorial-for-unreal-engine

AI/ML data versioning
- https://dvc.org/doc/user-guide/pipelines/run-cache
- https://doc.dvc.org/user-guide/project-structure/internal-files
- https://discuss.dvc.org/t/dvc-1-0-release/412
- https://lakefs.io/blog/lakefs-acquires-dvc/
- https://docs.lakefs.io/understand/model/
- https://docs.lakefs.io/understand/how/versioning-internals/
- https://lakefs.io/blog/concrete-graveler-committing-data-to-pebbledb-sstables/
- https://lakefs.io/lp-lakefs-mount/
- https://docs.lakefs.io/security/rbac/
- https://docs.lakefs.io/security/
- https://huggingface.co/docs/hub/xet
- https://github.com/huggingface/hub-docs/blob/main/docs/hub/xet/deduplication.md
- https://huggingface.co/docs/xet/deduplication.md
- https://huggingface.co/docs/xet/upload-protocol
- https://github.com/huggingface/xet-core
- https://huggingface.co/blog/from-files-to-chunks
- https://huggingface.co/blog/from-chunks-to-blocks
- https://www.cidrdb.org/cidr2023/slides/p43-low-slides.pdf
- https://huggingface.co/spaces/jsulz/ready-xet-go/discussions/1
- https://docs.pachyderm.com/2.8.x/learn/intro-data-versioning
- https://docs.pachyderm.com/2.8.x/learn/glossary/provenance
- https://pachyderm.com/blog/pachyderm-1-8-performance-improvements
- https://docs.dolthub.com/architecture/storage-engine
- https://dolthub.com/blog/2024-02-29-storage-engine
- https://lwn.net/Articles/1068864/
- https://infoq.com/news/2026/07/dolt-version-control
- https://docs.wandb.ai/models/support/artifact_storage_version
- https://docs.wandb.ai/models/ref/python/experiments/artifact
- https://wandb.ai/wandb/customers/reports/W-B-Artifacts-Overview--VmlldzoxMTE2NjM2Ng
- https://mlflow.org/docs/latest/ml/tracking/server
- https://mlflow.org/docs/latest/self-hosting/architecture/tracking-server/
- https://github.com/Oxen-AI/Oxen
- https://docs.oxen.ai/features/performance
- https://docs.oxen.ai/getting-started/versioning
- https://raw.githubusercontent.com/apache/iceberg/main/format/spec.md
- https://iceberg.apache.org/spec/
- https://www.dremio.com/blog/apache-iceberg-snapshot-expiration/
- https://iceberglakehouse.com/iceberg/iceberg-snapshot-references/
- https://raw.githubusercontent.com/delta-io/delta/master/PROTOCOL.md
- https://docs.databricks.com/aws/sql/language-manual/delta-vacuum
- https://huggingface.co/docs/hub/en/datasets-cards
- https://iclr.cc/virtual/2024/poster/17463
