# Changelog

## [0.2.0](https://github.com/maboloshi/hok/compare/v0.2.0-beta.3...v0.2.0) (2026-08-17)

### Features

* **cli:** add -a/--all to reset ([36bc5ed](https://github.com/maboloshi/hok/commit/36bc5ed))
* **cli:** add -a/--arch to depends ([cb80e34](https://github.com/maboloshi/hok/commit/cb80e34))
* **libscoop:** persist_permission - grant Users write access on global persist root ([8a93ada](https://github.com/maboloshi/hok/commit/8a93ada))
* **libscoop:** native extraction of PE-prefixed ZIP (SFX) payloads ([58e82ef](https://github.com/maboloshi/hok/commit/58e82ef))
* **libscoop:** extract NSIS uninstaller stub (Uninstall.exe) ([7aaef75](https://github.com/maboloshi/hok/commit/7aaef75))
* **cli:** hok shim add/rm/list/info with long help; shared shim logic ([f736222](https://github.com/maboloshi/hok/commit/f736222))
* **libscoop:** hok config edit honors editor config key ([984a4f2](https://github.com/maboloshi/hok/commit/984a4f2))
* **cli:** show "Everything is ok!" on sync success; confirm install -d ([6fe4b8e](https://github.com/maboloshi/hok/commit/6fe4b8e))
* **libscoop:** extract NSIS installers via the nsis crate ([c5b25ec](https://github.com/maboloshi/hok/commit/c5b25ec))
* **cli:** add download command ([ffee298](https://github.com/maboloshi/hok/commit/ffee298))
* **libscoop:** extract hash verification, add download_apps API ([3d2ce38](https://github.com/maboloshi/hok/commit/3d2ce38))
* **libscoop:** pick shim variant from target PE subsystem ([fb5eef6](https://github.com/maboloshi/hok/commit/fb5eef6))
* **hok-shim:** split into console/GUI variants, select by target subsystem ([4910231](https://github.com/maboloshi/hok/commit/4910231))
* **cli:** case-insensitive subcommand names ([5e4053a](https://github.com/maboloshi/hok/commit/5e4053a))
* **libscoop:** workspace dir follows effective root (global-aware) ([880ee63](https://github.com/maboloshi/hok/commit/880ee63))
* **libscoop:** install URL/path/@version manifests as isolated packages ([bedf396](https://github.com/maboloshi/hok/commit/bedf396))
* **libscoop:** resolve and generate manifests for isolated installs ([421dfd0](https://github.com/maboloshi/hok/commit/421dfd0))
* **libscoop:** parse install queries (app@version / URL / local path) ([ae16f46](https://github.com/maboloshi/hok/commit/ae16f46))
* **shim:** align shim generation with upstream Scoop ([127f2e2](https://github.com/maboloshi/hok/commit/127f2e2))
* **checkver:** apply full set of autoupdate generic properties ([403ea7b](https://github.com/maboloshi/hok/commit/403ea7b))
* **bucket:** add `bucket known` subcommand (P3-12) ([45cef93](https://github.com/maboloshi/hok/commit/45cef93))
* **libscoop:** generate known buckets from buckets.json at build time (P3-11) ([b81e120](https://github.com/maboloshi/hok/commit/b81e120))
* **libscoop:** rewrite nightly to nightly-YYYYMMDD for the install layout (P3-8) ([988130a](https://github.com/maboloshi/hok/commit/988130a))
* accept Scoop flag aliases for cache/hash (P3-2, P3-3) ([de75eed](https://github.com/maboloshi/hok/commit/de75eed))
* **libscoop:** record url in install.json (P3-7) ([9e59e47](https://github.com/maboloshi/hok/commit/9e59e47))
* **formatjson:** align schema.json structural checks with official gate ([de73c2a](https://github.com/maboloshi/hok/commit/de73c2a))
* **libscoop:** run installer.file and installer.script together (P1-8) ([bebb668](https://github.com/maboloshi/hok/commit/bebb668))
* **install:** add -a/--arch to override the effective architecture ([ce3c451](https://github.com/maboloshi/hok/commit/ce3c451))
* **libscoop:** support bin object form; drop unparseable entries (P0-6) ([7f13809](https://github.com/maboloshi/hok/commit/7f13809))
* **libscoop:** tolerate string-form autoupdate.hash (P0-5) ([872ea19](https://github.com/maboloshi/hok/commit/872ea19))
* **libscoop:** tolerate missing required manifest fields (P0-2) ([0e26cde](https://github.com/maboloshi/hok/commit/0e26cde))
* **libscoop:** undo installer PATH pollution and honor installer.keep ([a5faf4c](https://github.com/maboloshi/hok/commit/a5faf4c))
* **libscoop:** install psmodule via junction + PSModulePath ([5a5819f](https://github.com/maboloshi/hok/commit/5a5819f))
* **config:** expose supported settings with defaults in help/list/README ([b822541](https://github.com/maboloshi/hok/commit/b822541))
* **cmd:** stream checkver/checkurls/checkhashes output per app ([58a3c96](https://github.com/maboloshi/hok/commit/58a3c96))
* **package:** align formatjson with official Scoop formatjson.ps1 ([36f4717](https://github.com/maboloshi/hok/commit/36f4717))
* **process-check:** align running-process detection with Scoop ([ab4b3ce](https://github.com/maboloshi/hok/commit/ab4b3ce))
* **config:** introduce hok-owned config file with one-time Scoop migration ([c351242](https://github.com/maboloshi/hok/commit/c351242))
* **download:** report per-chunk progress for fragmented downloads ([d517bbd](https://github.com/maboloshi/hok/commit/d517bbd))
* **download:** re-download failed chunk ranges serially instead of failing ([142e402](https://github.com/maboloshi/hok/commit/142e402))
* **download:** retry chunk downloads with exponential backoff ([e3acc1a](https://github.com/maboloshi/hok/commit/e3acc1a))
* **libscoop|env:** apply env_set/env_add_path during install ([5d92660](https://github.com/maboloshi/hok/commit/5d92660))
* **checkver:** align remaining Scoop behaviors from main f60bbb6 ([5ff114d](https://github.com/maboloshi/hok/commit/5ff114d))
* **install:** sink suggest display into libscoop, align with Scoop ([9f909f6](https://github.com/maboloshi/hok/commit/9f909f6))
* **manifest:** align manifest parsing with original Scoop ([01594fc](https://github.com/maboloshi/hok/commit/01594fc))
* **config:** support default_architecture config override ([41457ac](https://github.com/maboloshi/hok/commit/41457ac))
* **arch:** add runtime OS architecture detection ([fe8af81](https://github.com/maboloshi/hok/commit/fe8af81))

### Bug Fixes

* **cli:** reset -a keeps resetting remaining apps after a failure ([14541c7](https://github.com/maboloshi/hok/commit/14541c7))
* **libscoop:** write shortcut arguments into .lnk ([5beedb7](https://github.com/maboloshi/hok/commit/5beedb7))
* **libscoop:** parse 7z.exe progress output ([b268f63](https://github.com/maboloshi/hok/commit/b268f63))
* throttle extraction progress; clear line and show counter, not file names ([d1865f8](https://github.com/maboloshi/hok/commit/d1865f8))
* **libscoop:** filter running apps before assembling upgrade transaction ([57e23ae](https://github.com/maboloshi/hok/commit/57e23ae))
* **libscoop:** define $fname for package scripts (installer remote filename) ([2207a8c](https://github.com/maboloshi/hok/commit/2207a8c))
* **libscoop:** raise NSIS decompression budget to 512 MiB for large solid installers ([a1e99f0](https://github.com/maboloshi/hok/commit/a1e99f0))
* **libscoop:** recognize FirstHeader-only NSIS stubs (AltSnap) ([298eae9](https://github.com/maboloshi/hok/commit/298eae9))
* **libscoop:** NSIS extraction tracks EW_CREATEDIR directories (7-Zip parity) ([1db3bc4](https://github.com/maboloshi/hok/commit/1db3bc4))
* **libscoop:** tolerate missing HOME/APPDATA and non-UTF-8 names ([0b44e75](https://github.com/maboloshi/hok/commit/0b44e75))
* **cli:** hok update --force no longer panics with up-to-date apps ([6d9050d](https://github.com/maboloshi/hok/commit/6d9050d))
* **libscoop:** update --force reinstalls current version; download ends loop on any error ([453c459](https://github.com/maboloshi/hok/commit/453c459))
* **cli:** update --force stays installed-only; declined confirm aborts; EOF-safe prompts ([110bc06](https://github.com/maboloshi/hok/commit/110bc06))
* **libscoop:** download empty-set ends the event loop ([157aaf1](https://github.com/maboloshi/hok/commit/157aaf1))
* **cli:** restore cursor on error exit; warn on unreadable bucket ([663423b](https://github.com/maboloshi/hok/commit/663423b))
* **libscoop:** close the resolve and sizing event phases ([1ebedbc](https://github.com/maboloshi/hok/commit/1ebedbc))
* **libscoop:** download reports failures via non-zero exit ([ce59d5a](https://github.com/maboloshi/hok/commit/ce59d5a))
* **cli:** non-interactive prompts no longer hang ([88366a4](https://github.com/maboloshi/hok/commit/88366a4))
* **libscoop:** skip manifest cache entry on field serialization failure ([62a219a](https://github.com/maboloshi/hok/commit/62a219a))
* **libscoop:** ignore_failures defaults to false ([53e35f3](https://github.com/maboloshi/hok/commit/53e35f3))
* **libscoop:** don't treat HEAD failures as offline ([1b30aaa](https://github.com/maboloshi/hok/commit/1b30aaa))
* **cli:** download command no longer prints "all apps are up to date" ([80041be](https://github.com/maboloshi/hok/commit/80041be))
* **libscoop:** restore fragmented download and progress bars ([970e5bd](https://github.com/maboloshi/hok/commit/970e5bd))
* **libscoop:** reject non-string checkver hash extraction fields ([26e0ec9](https://github.com/maboloshi/hok/commit/26e0ec9))
* **libscoop:** show explicit placeholder for out-of-range url in checkhashes ([1b980c1](https://github.com/maboloshi/hok/commit/1b980c1))
* **libscoop:** fail closed when process enumeration fails ([378007c](https://github.com/maboloshi/hok/commit/378007c))
* **libscoop:** unify URL filename extraction ([06df7de](https://github.com/maboloshi/hok/commit/06df7de))
* **libscoop:** allow bucket pull when remote adds new files ([2d3bbb7](https://github.com/maboloshi/hok/commit/2d3bbb7))
* **libscoop:** case-insensitive package and bucket resolution ([dbc5b23](https://github.com/maboloshi/hok/commit/dbc5b23))
* **libscoop:** emit boolean literal for $global in script preamble ([4b27d92](https://github.com/maboloshi/hok/commit/4b27d92))
* **libscoop:** don't treat NSIS/Inno installers as 7z SFX ([e00bc71](https://github.com/maboloshi/hok/commit/e00bc71))
* **shim:** skip empty args line for two-element bin entries ([b899fb3](https://github.com/maboloshi/hok/commit/b899fb3))
* **manifest:** coerce non-string env_set/cookie values to strings ([f440296](https://github.com/maboloshi/hok/commit/f440296))
* **checkver:** make sourceforge path optional and align string form ([8835786](https://github.com/maboloshi/hok/commit/8835786))
* **manifest:** relax hash pattern to official combination lengths ([7724424](https://github.com/maboloshi/hok/commit/7724424))
* **checkver:** tolerate hook-script object form and named-group replace ([4668cca](https://github.com/maboloshi/hok/commit/4668cca))
* **checkver:** select version group with Scoop group-name semantics ([6e58162](https://github.com/maboloshi/hok/commit/6e58162))
* **checkver:** chain custom regex onto jsonpath result for GitHub checkvers ([aca9f35](https://github.com/maboloshi/hok/commit/aca9f35))
* **libscoop:** record the selected architecture in install.json and scripts (P3-5/6) ([bff7217](https://github.com/maboloshi/hok/commit/bff7217))
* **libscoop:** write last_update unconditionally after bucket sync (P3-14) ([f339937](https://github.com/maboloshi/hok/commit/f339937))
* **libscoop:** never panic on an empty bin def (P3-19) ([7658d47](https://github.com/maboloshi/hok/commit/7658d47))
* **libscoop:** delete corrupt cache on hash mismatch (P3-1) ([1242373](https://github.com/maboloshi/hok/commit/1242373))
* **libscoop:** allow updating the running hok itself (P1-10) ([760a07b](https://github.com/maboloshi/hok/commit/760a07b))
* **libscoop:** tolerate BOM and JSON5 in manifest/config loading (P0-3) ([dfdf203](https://github.com/maboloshi/hok/commit/dfdf203))
* **libscoop:** honor installer exit code (P0-4) ([6f76a8b](https://github.com/maboloshi/hok/commit/6f76a8b))
* **libscoop:** skip empty-hash verification (P0-1) ([291bfba](https://github.com/maboloshi/hok/commit/291bfba))
* **update:** -f/--force matches Scoop; --ignore-failure long-only + i18n ([6755288](https://github.com/maboloshi/hok/commit/6755288))
* **libscoop:** script $dir/$original_dir/$global parity with Scoop ([3a127d2](https://github.com/maboloshi/hok/commit/3a127d2))
* **libscoop:** append .lnk/.shim extensions verbatim for dotted names ([e44bd4f](https://github.com/maboloshi/hok/commit/e44bd4f))
* **checkver:** support jsonpath field names with special characters ([62c874d](https://github.com/maboloshi/hok/commit/62c874d))
* **checkver:** pass proxy to checkver.script via HTTP_PROXY/HTTPS_PROXY ([02aeab5](https://github.com/maboloshi/hok/commit/02aeab5))
* **checkver:** run checkver.script items concurrently with downloads ([0f678e3](https://github.com/maboloshi/hok/commit/0f678e3))
* **fs:** write_json outputs CRLF with trailing newline (align Scoop) ([b251987](https://github.com/maboloshi/hok/commit/b251987))
* **libscoop:** expand `$dir`/`$persist_dir` placeholders in post-install notes ([a17df6f](https://github.com/maboloshi/hok/commit/a17df6f))
* **ui:** show env add/set progress output ([02cb9da](https://github.com/maboloshi/hok/commit/02cb9da))
* **download:** guard chunk splitting against small files, tighten part check ([a83d359](https://github.com/maboloshi/hok/commit/a83d359))
* **libscoop:** align uninstall flow with scoop-uninstall.ps1 ([515f540](https://github.com/maboloshi/hok/commit/515f540))
* **libscoop:** resolve package paths via effective_root_path ([af5abfe](https://github.com/maboloshi/hok/commit/af5abfe))
* **reset:** resolve broken installs directly from disk, matching upstream ([bd50474](https://github.com/maboloshi/hok/commit/bd50474))
* **extract:** recognize .iso archives and strip URL query from filenames ([a77947a](https://github.com/maboloshi/hok/commit/a77947a))
* **shim:** align shim conflict handling with upstream warn_on_overwrite ([f893e3a](https://github.com/maboloshi/hok/commit/f893e3a))
* **shortcut:** warn only when overwriting another package's shortcut ([3c5f321](https://github.com/maboloshi/hok/commit/3c5f321))
* **sync:** error on empty package resolution, accept backslash separators ([dace960](https://github.com/maboloshi/hok/commit/dace960))
* **cache:** tolerate locked or missing cache files in list/remove ([47c628e](https://github.com/maboloshi/hok/commit/47c628e))
* **uninstall:** skip missing packages and dedupe output under IgnoreFailure ([cf6c634](https://github.com/maboloshi/hok/commit/cf6c634))
* **sync:** honor IgnoreFailure across the whole install/update pipeline ([57802c0](https://github.com/maboloshi/hok/commit/57802c0))
* **cli:** drop duplicate --quiet flag that panicked update/upgrade parsing ([d8d21c8](https://github.com/maboloshi/hok/commit/d8d21c8))
* **error:** unify anyhow to crate::Error in virustotal and formatjson ([9a5d301](https://github.com/maboloshi/hok/commit/9a5d301))
* **checkver:** decouple UI from libscoop ([3054452](https://github.com/maboloshi/hok/commit/3054452))
* repair pre-existing test compile error and invalid regex ([05e28d2](https://github.com/maboloshi/hok/commit/05e28d2))
* suppress dead_code warning on strip_url_fragment, app_filter_matches ([643c1f8](https://github.com/maboloshi/hok/commit/643c1f8))
* extract remaining business logic from cmd/virustotal.rs, cmd/formatjson.rs, cmd/checkup.rs ([f80f47d](https://github.com/maboloshi/hok/commit/f80f47d))
* Fix rayon Session sync issue in package query ([ed9f5e1](https://github.com/maboloshi/hok/commit/ed9f5e1))
* correct invalid hash lengths in test fixtures and inline test data ([fd7e2c3](https://github.com/maboloshi/hok/commit/fd7e2c3))
* suppress dead_code warning on extract_name_and_bucket ([a98b087](https://github.com/maboloshi/hok/commit/a98b087))
* initialize rust_i18n in libscoop crate root ([6572eaa](https://github.com/maboloshi/hok/commit/6572eaa))

### Performance Improvements

* **libscoop:** parallel SFX dispatch, Arc-shared buffer, smarter zip workers ([5fa74c3](https://github.com/maboloshi/hok/commit/5fa74c3))
* **libscoop:** parallel zip extraction (per-entry worker pool) ([379b7b0](https://github.com/maboloshi/hok/commit/379b7b0))
* **libscoop:** decode tar.xz with lzma-rust2 (1.5x faster, streaming) ([c1643bb](https://github.com/maboloshi/hok/commit/c1643bb))
* **libscoop:** block-parallel 7z decode ([8812596](https://github.com/maboloshi/hok/commit/8812596))

### Code Refactoring

* **libscoop:** adopt upstream nsis 0.4.0, retire vendored fork ([9766e7b](https://github.com/maboloshi/hok/commit/9766e7b))
* **libscoop:** single-pass installer classifier; Inno SFX fallback ([5cb14a6](https://github.com/maboloshi/hok/commit/5cb14a6))
* **libscoop:** split query and download into focused modules ([1396807](https://github.com/maboloshi/hok/commit/1396807))
* **libscoop:** clap Args -> plain domain Options (checkver, checkhashes) ([a7fc8b2](https://github.com/maboloshi/hok/commit/a7fc8b2))
* **libscoop:** internal modules are pub(crate); Session::set_default_architecture ([97aa95b](https://github.com/maboloshi/hok/commit/97aa95b))
* **libscoop:** sync entry wraps install/remove with event closure ([31ea1ee](https://github.com/maboloshi/hok/commit/31ea1ee))
* **cli:** reinstall ok_all; drop dead progress param; panic-free shim sort; validate_dir errors ([fd5a33c](https://github.com/maboloshi/hok/commit/fd5a33c))
* **cli:** drop dead show_progress_bars flag; close download event rendering ([cf36291](https://github.com/maboloshi/hok/commit/cf36291))
* **cli:** move "all apps are up to date" to the update command ([7e9306a](https://github.com/maboloshi/hok/commit/7e9306a))
* **libscoop:** remove dead code and stale reference blocks ([8cb5c9f](https://github.com/maboloshi/hok/commit/8cb5c9f))
* **cli:** remove unused Cmd trait scaffolding ([085f629](https://github.com/maboloshi/hok/commit/085f629))
* **libscoop:** remove dead Manifest.hash ([4218ca4](https://github.com/maboloshi/hok/commit/4218ca4))
* **libscoop:** extract isolated query dispatch and download/verify sequence ([3925cea](https://github.com/maboloshi/hok/commit/3925cea))
* **libscoop:** extract installer/uninstaller hook runner and bucket matcher ([35dde13](https://github.com/maboloshi/hok/commit/35dde13))
* apply cargo fmt across the workspace ([9fc50d6](https://github.com/maboloshi/hok/commit/9fc50d6))
* **libscoop:** align bucket update with official git pull semantics ([cd24c81](https://github.com/maboloshi/hok/commit/cd24c81))
* **libscoop:** move bucket updated_at formatting into internal::time ([3b326f8](https://github.com/maboloshi/hok/commit/3b326f8))
* **libscoop:** pair last_update time codec and drop jiff for time 0.3 (P1-22) ([d181d22](https://github.com/maboloshi/hok/commit/d181d22))
* **libscoop:** unify GitHub/SourceForge URL parsing in internal::url (P1-19) ([72e80c9](https://github.com/maboloshi/hok/commit/72e80c9))
* **libscoop:** complete no_junction support (P1-18) ([6ef3f70](https://github.com/maboloshi/hok/commit/6ef3f70))
* **libscoop:** unify ps_command/scoop_arch and full-var expansion ([6014108](https://github.com/maboloshi/hok/commit/6014108))
* **query:** simplify install-state path building in query.rs ([827d7c4](https://github.com/maboloshi/hok/commit/827d7c4))
* **paths:** consolidate Scoop layout dirs into Session methods ([31ea057](https://github.com/maboloshi/hok/commit/31ea057))
* **libscoop:** rename expand_installer_vars to expand_scoop_vars ([168ebf4](https://github.com/maboloshi/hok/commit/168ebf4))
* **libscoop:** rustfmt config.rs and manifest.rs (pre-existing format drift) ([8fb73ca](https://github.com/maboloshi/hok/commit/8fb73ca))
* **output:** route all libscoop UI output through session sink (P0-11) ([7a97cc8](https://github.com/maboloshi/hok/commit/7a97cc8))
* **package:** unify installer/uninstaller file execution via run_installer_file ([e4944dd](https://github.com/maboloshi/hok/commit/e4944dd))
* **cmd:** unify directory validation via validate_dir (i18n) ([23e156b](https://github.com/maboloshi/hok/commit/23e156b))
* **package:** move formatjson scanning into libscoop via format_manifests ([1bb88fb](https://github.com/maboloshi/hok/commit/1bb88fb))
* **package:** unify checkhashes/checkver/checkurls on shared primitives ([8e0a2db](https://github.com/maboloshi/hok/commit/8e0a2db))
* **package:** add shared hash/json/manifest-discovery primitives ([ede27a2](https://github.com/maboloshi/hok/commit/ede27a2))
* **network:** unify PRIVATE_HOSTS header injection via match_private_hosts ([399b5a1](https://github.com/maboloshi/hok/commit/399b5a1))
* **cmd:** unify --global admin checks via ensure_global (i18n) ([b5169fd](https://github.com/maboloshi/hok/commit/b5169fd))
* **cmd:** merge hold/unhold and align with official Scoop behavior ([c6364b7](https://github.com/maboloshi/hok/commit/c6364b7))
* **libscoop:** drop operations wrappers, call persist/shortcut directly ([8336e80](https://github.com/maboloshi/hok/commit/8336e80))
* **cmd:** centralize sync flags and unify command module style ([2f09973](https://github.com/maboloshi/hok/commit/2f09973))
* **libscoop:** move run_script/expand_installer_vars into operations::script ([68ebb75](https://github.com/maboloshi/hok/commit/68ebb75))
* **libscoop:** add section comments to operations module ([0adee83](https://github.com/maboloshi/hok/commit/0adee83))
* **cli:** move shell-open helpers into libscoop::os, drop util.rs ([018137b](https://github.com/maboloshi/hok/commit/018137b))
* **cli:** extract formatting helpers into hok::format ([dd1546f](https://github.com/maboloshi/hok/commit/dd1546f))
* **libscoop:** move compare_versions into internal::version ([5cd0f63](https://github.com/maboloshi/hok/commit/5cd0f63))
* **libscoop:** extract package identity helpers into package::identity ([d4554dd](https://github.com/maboloshi/hok/commit/d4554dd))
* **libscoop:** unify is_version_dir into internal::path ([ae83883](https://github.com/maboloshi/hok/commit/ae83883))
* **libscoop:** add internal::string module and unify glob/encoding utils ([5d971fd](https://github.com/maboloshi/hok/commit/5d971fd))
* merge scoop-hash into libscoop internal::hash ([4267de1](https://github.com/maboloshi/hok/commit/4267de1))
* **extract:** delegate archive detection to archive::detect_format ([8065fc0](https://github.com/maboloshi/hok/commit/8065fc0))
* converge layering — move system calls into libscoop, block internal cross-layer refs ([b7079f1](https://github.com/maboloshi/hok/commit/b7079f1))
* split sync.rs, checkver.rs and manifest.rs into peer modules ([f166d19](https://github.com/maboloshi/hok/commit/f166d19))
* clean dead code and unify duplicate implementations ([de346ca](https://github.com/maboloshi/hok/commit/de346ca))
* **operations:** split mod.rs into per-category modules ([c8e87d7](https://github.com/maboloshi/hok/commit/c8e87d7))
* **sync:** extract file operation primitives to operations/ ([60a8d30](https://github.com/maboloshi/hok/commit/60a8d30))
* **operation:** remove facade ([e28b2eb](https://github.com/maboloshi/hok/commit/e28b2eb))
* **error:** unify error types in package modules ([3593ea4](https://github.com/maboloshi/hok/commit/3593ea4))
* **network:** unify network layer interface with RequestOptions ([0a16408](https://github.com/maboloshi/hok/commit/0a16408))
* rustfmt migrated arch/manifest code ([20bb210](https://github.com/maboloshi/hok/commit/20bb210))
* sink list/depends/shim business logic into libscoop ([f12b447](https://github.com/maboloshi/hok/commit/f12b447))
* sink create/import/export business logic into libscoop ([caae36f](https://github.com/maboloshi/hok/commit/caae36f))
* drop unused bucket manifest parser ([e7c59e0](https://github.com/maboloshi/hok/commit/e7c59e0))
* use effective root path in operations ([fc4d11f](https://github.com/maboloshi/hok/commit/fc4d11f))
* unify package query matching helpers ([23ebf3a](https://github.com/maboloshi/hok/commit/23ebf3a))
* centralize shared URL helpers ([38a87b3](https://github.com/maboloshi/hok/commit/38a87b3))
* reuse shared manifest discovery ([2a7a7a6](https://github.com/maboloshi/hok/commit/2a7a7a6))
* add shared package manifest walker ([b1bbd68](https://github.com/maboloshi/hok/commit/b1bbd68))
* step 7 - move auto_pr core logic to libscoop, extract GitHub client ([68a9cec](https://github.com/maboloshi/hok/commit/68a9cec))
* step 7 - switch remaining checkver callsites to libscoop package ([cd89871](https://github.com/maboloshi/hok/commit/cd89871))
* step 6 - extract missing_checkver into libscoop package module ([8e75cf0](https://github.com/maboloshi/hok/commit/8e75cf0))
* step 5 - extract checkver into libscoop package module ([6e022b5](https://github.com/maboloshi/hok/commit/6e022b5))
* step 4 - extract checkhashes into libscoop package module ([7ad34bf](https://github.com/maboloshi/hok/commit/7ad34bf))
* step 3 - extract checkurls into libscoop package module ([23e52ad](https://github.com/maboloshi/hok/commit/23e52ad))
* step 2 - move extract_name_and_bucket to bucket.rs ([f8e190b](https://github.com/maboloshi/hok/commit/f8e190b))

### Documentation

* **libscoop:** align sync.rs and query_synced_cached docs with reality ([c3f9c8c](https://github.com/maboloshi/hok/commit/c3f9c8c))
* **hok-shim:** document dual-variant shim selection ([3d59426](https://github.com/maboloshi/hok/commit/3d59426))
* **hok:** align install help with upstream usage forms ([a8c56ff](https://github.com/maboloshi/hok/commit/a8c56ff))
* **libscoop:** clear the remaining 16 rustdoc warnings (private links + bare URLs) ([6915f18](https://github.com/maboloshi/hok/commit/6915f18))
* **libscoop:** fix 11 broken intra-doc links so cargo doc builds cleanly ([1dae3ca](https://github.com/maboloshi/hok/commit/1dae3ca))
* **libscoop:** document the two output channels (sink + event bus) in crate/session docs ([0b28a50](https://github.com/maboloshi/hok/commit/0b28a50))
* translate Chinese doc comments to English ([3773745](https://github.com/maboloshi/hok/commit/3773745))

### Miscellaneous Chores

* remove hok-shim-ref reference crate ([d7ade5a](https://github.com/maboloshi/hok/commit/d7ade5a))
* **libscoop:** cover persist_permission ACL grant idempotency ([458aa8c](https://github.com/maboloshi/hok/commit/458aa8c))
* ignore .dsh workspace directory ([e25af94](https://github.com/maboloshi/hok/commit/e25af94))
* regenerate Cargo.lock ([161f311](https://github.com/maboloshi/hok/commit/161f311))
* **crates/innospect:** align vendored crate to upstream 0.1.3 ([e3c1ec1](https://github.com/maboloshi/hok/commit/e3c1ec1))
* **crates/innospect:** lzma-rust2 + zlib-rs decoders; ureq default-features off (drop miniz_oxide) ([d629b70](https://github.com/maboloshi/hok/commit/d629b70))
* **crates/nsis:** use zlib-rs flate2 backend for deflate decoding ([eb44383](https://github.com/maboloshi/hok/commit/eb44383))
* **crates/nsis:** nsis-rs fork with lzma-rust2 decoder ([93fb7f6](https://github.com/maboloshi/hok/commit/93fb7f6))
* **libscoop:** bump sevenz-rust2/lzma-rust2; extend 7z perf benchmark diagnostics ([0c05848](https://github.com/maboloshi/hok/commit/0c05848))
* **libscoop:** clippy fixes and dead code removal ([05af215](https://github.com/maboloshi/hok/commit/05af215))
* dev profile reduces debug info size ([f2cadda](https://github.com/maboloshi/hok/commit/f2cadda))
* **libscoop:** verify NSIS extraction against electron-builder installer ([8156e71](https://github.com/maboloshi/hok/commit/8156e71))
* Revert "feat(libscoop): workspace dir follows effective root (global-aware)" ([32cd898](https://github.com/maboloshi/hok/commit/32cd898))
* ignore .reasonix workspace directory ([3886a25](https://github.com/maboloshi/hok/commit/3886a25))
* **download:** cover chunk retry/resume/Range edges with a mock server ([fd2ccd4](https://github.com/maboloshi/hok/commit/fd2ccd4))
* **utils:** pin cache_path in test_session to <root>/cache ([b6ce79e](https://github.com/maboloshi/hok/commit/b6ce79e))
* **extract:** cover archive extraction orchestration with unit tests ([3086de7](https://github.com/maboloshi/hok/commit/3086de7))
* trim redundant deps and slim archive feature set ([350c8c9](https://github.com/maboloshi/hok/commit/350c8c9))
* **package:** add unit tests for resolve, cleanup, hold ([90c22cd](https://github.com/maboloshi/hok/commit/90c22cd))
* fix all clippy warnings in the workspace ([0f52785](https://github.com/maboloshi/hok/commit/0f52785))
* format code with rustfmt and fix clippy warnings ([051df51](https://github.com/maboloshi/hok/commit/051df51))
* add unit tests for manifest_walker, checkurls, checkhashes, checkver, query ([080be07](https://github.com/maboloshi/hok/commit/080be07))
* Apply remaining changes ([9717019](https://github.com/maboloshi/hok/commit/9717019))
* Add write permissions to release workflow ([db4ccba](https://github.com/maboloshi/hok/commit/db4ccba))
## [0.2.0-beta.3](https://github.com/maboloshi/hok/compare/v0.2.0-beta.2...v0.2.0-beta.3) (2026-07-30)

### Features

* **checkver:** Align with Scoop — script mode, github detection, hash modes, and more
  ([6468bb3](https://github.com/maboloshi/hok/commit/6468bb3))
* **checkurls:** Full alignment with original Scoop checkurls.ps1 — recursive scan, timeout,
  error reporting ([af0294d](https://github.com/maboloshi/hok/commit/af0294d))
* **checkhashes:** Full alignment with original Scoop checkhashes.ps1 — hash verification
  and update ([3a7bd05](https://github.com/maboloshi/hok/commit/3a7bd05))
* **formatjson:** Recursive scan, glob filtering, extract shared walkdir_files helper
  ([9d0d017](https://github.com/maboloshi/hok/commit/9d0d017))
* **ci-auto-pr:** New `hok ci-auto-pr` subcommand for GitHub API-based auto PR workflow
  ([1f88c9b](https://github.com/maboloshi/hok/commit/1f88c9b))
* **install:** Align with upstream scoop-install behavior
  ([d7cd23b](https://github.com/maboloshi/hok/commit/d7cd23b))
* **uninstall:** Align with upstream and add running-process guard
  ([3149095](https://github.com/maboloshi/hok/commit/3149095))
* **update,upgrade,reinstall:** Add `--global` and `-s` short flag
  ([8c7f058](https://github.com/maboloshi/hok/commit/8c7f058))
* **update,upgrade:** Align with upstream scoop-update.ps1
  ([2a845d2](https://github.com/maboloshi/hok/commit/2a845d2))

### Bug Fixes

* **archive:** Resolve path traversal, unreachable panic, 7z memory, and i18n issues
  ([9e9fbde](https://github.com/maboloshi/hok/commit/9e9fbde))
* **libscoop:** Correct regex patterns in constant.rs
  ([d750141](https://github.com/maboloshi/hok/commit/d750141))
* **libscoop:** Address code review findings in operation.rs
  ([5ee3b7f](https://github.com/maboloshi/hok/commit/5ee3b7f))
* **libscoop(bucket):** Log warning instead of silently dropping failed directory entries
  in par_read_dir ([972020d](https://github.com/maboloshi/hok/commit/972020d))
* **shim:** Resolve alt-filename inconsistency and .exe conflict handling
  ([d0d9c89](https://github.com/maboloshi/hok/commit/d0d9c89))
* **hok-shim:** Use `ALTERNATENAME` to avoid LLD duplicate symbol errors
  ([d89f24e](https://github.com/maboloshi/hok/commit/d89f24e))
* **session:** Log an `output::warn` when all configuration paths fail to load and
  revert to default config ([57a1be4](https://github.com/maboloshi/hok/commit/57a1be4))
* **shortcut:** Improved deletion error handling and shared shortcut conflict warnings
  ([3871515](https://github.com/maboloshi/hok/commit/3871515))

### Code Refactoring

* **command framework:** Migrate all commands to `Cmd` trait + `SyncArgs`
  ([5694ef1](https://github.com/maboloshi/hok/commit/5694ef1))
* **event/output:** Decouple event and output systems
  ([b05d053](https://github.com/maboloshi/hok/commit/b05d053))
* **command framework:** Infrastructure for future command extensions
  ([33faa5f](https://github.com/maboloshi/hok/commit/33faa5f))
* **formatjson:** Replace JSON text patching with direct `Value` manipulation
  ([3c6fe59](https://github.com/maboloshi/hok/commit/3c6fe59))
* **sync:** Extract `confirm_transaction`, add `TempFileGuard` Drop guard
  ([7901cd0](https://github.com/maboloshi/hok/commit/7901cd0))
* **sync:** Clean up shadowed `manifest_src` variable
  ([ae4fd56](https://github.com/maboloshi/hok/commit/ae4fd56))
* **sync:** Remove redundant `has_install_script` gating in `commit_one_install`
  ([5fbaf59](https://github.com/maboloshi/hok/commit/5fbaf59))
* **sync:** Deduplicate `root_dir`/`apps_dir` via `session.effective_root_path()`
  ([eb150fa](https://github.com/maboloshi/hok/commit/eb150fa))
* **sync:** Extract `check_not_running` helper, deduplicate running-process guard
  ([972f311](https://github.com/maboloshi/hok/commit/972f311))
* **install:** Move `prune_installed` from `cmd/install` to `libscoop::operation`
  ([c276e6b](https://github.com/maboloshi/hok/commit/c276e6b))
* **checkver/util:** Add section markers to checkver.rs and util.rs
  ([8ef688d](https://github.com/maboloshi/hok/commit/8ef688d))

### Documentation

* Add standardized `//!` module header documentation for 34 source files
  ([c85995d](https://github.com/maboloshi/hok/commit/c85995d))
* **eventloop:** Add comprehensive extension guide to module header
  ([db5b909](https://github.com/maboloshi/hok/commit/db5b909))
* **shim:** Annotate known gaps vs Scoop as TODO comments
  ([215c21f](https://github.com/maboloshi/hok/commit/215c21f))

### Miscellaneous Chores

* **search:** Use `QueryArgs::to_query_options()` in search.rs
  ([5e7098b](https://github.com/maboloshi/hok/commit/5e7098b))

## [0.2.0-beta.2](https://github.com/maboloshi/hok/compare/v0.2.0-beta.1...v0.2.0-beta.2) (2026-07-28)

### Features

* **format-json:** New `hok formatjson` command — lenient manifest parser,
  4-space indent, CRLF line endings, preserve field order ([1f52037](https://github.com/maboloshi/hok/commit/1f52037))
* **format-json:** Preserve JSON formatting when updating hashes/versions
  (text patching, no AST round-trip)

### Code Refactoring

* **deps:** Replace `unarc-rs` with `unrar` + 7z.exe fallback — eliminate
  duplicate `sevenz-rust2 v0.20` + `zip v8`, remove entire stale crypto chain
  ([fde869f](https://github.com/maboloshi/hok/commit/fde869f))
* **deps:** Upgrade `thiserror` v1+v2 → unified v2, `md-5`/`sha1`/`sha2`
  v0.10+v0.11 → unified v0.11 ([57429e3](https://github.com/maboloshi/hok/commit/57429e3), [e3ca3df](https://github.com/maboloshi/hok/commit/e3ca3df))
* **deps:** Remove `remove_dir_all` dependency — use
  `std::fs::remove_dir_all` (Rust 1.74+) ([4622c53](https://github.com/maboloshi/hok/commit/4622c53))
* **shim:** Replace string-based shim command with `clap Subcommand`
  ([7aff6cb](https://github.com/maboloshi/hok/commit/7aff6cb))
* **cat:** Remove `bat.exe` external dependency, always print directly
  ([a93a72d](https://github.com/maboloshi/hok/commit/a93a72d))
* **os:** Remove dead unix cfg from `running_apps` — project is Windows-only
  ([83fac96](https://github.com/maboloshi/hok/commit/83fac96))
* **fs:** Remove dead code from `fs.rs` ([f86543a](https://github.com/maboloshi/hok/commit/f86543a))
* **os:** Extract `is_pwsh_available` into shared `libscoop::internal::os`
  ([5735be4](https://github.com/maboloshi/hok/commit/5735be4))
* **code dedup:** Macroize 9 Manifest accessor methods via `arch_accessor!`,
  ChecksumBuilder methods, 3 `is_default_*` methods; consolidate 4 benchmark
  files into 1; deduplicate `encode_wide`/`open_file`/`open_url`;
  extract `compute_file_hash` to `scoop_hash`; extract shared `tmpdir()` test
  helper; merge `BucketUpdateUI::add`/`succeed` via `set_state` helper
  ([473cb90](https://github.com/maboloshi/hok/commit/473cb90), [6256dc1](https://github.com/maboloshi/hok/commit/6256dc1), [40a6709](https://github.com/maboloshi/hok/commit/40a6709),
  [682131d](https://github.com/maboloshi/hok/commit/682131d), [b2e228d](https://github.com/maboloshi/hok/commit/b2e228d), [69ee0ba](https://github.com/maboloshi/hok/commit/69ee0ba),
  [9a1aa6f](https://github.com/maboloshi/hok/commit/9a1aa6f), [fc73aa3](https://github.com/maboloshi/hok/commit/fc73aa3), [f2e202f](https://github.com/maboloshi/hok/commit/f2e202f))
* **archive:** Route ISO files directly to 7z.exe, remove `needs_fallback`
  ([dfb3bc2](https://github.com/maboloshi/hok/commit/dfb3bc2))

### Bug Fixes

* **archive:** `extract_tar` with `extract_dir` now writes to stripped path
  instead of original ([cd52246](https://github.com/maboloshi/hok/commit/cd52246))
* **alias:** Fix i18n for add/remove messages, fix UTF-8 truncation
  ([6c1df4c](https://github.com/maboloshi/hok/commit/6c1df4c))
* **checkver:** Show app name in error messages, strip v-prefix correctly,
  respect `--timeout`, add PowerShell timeout, uniform indent
  ([00cee15](https://github.com/maboloshi/hok/commit/00cee15))
* **checkurls:** Ensure newline before error output for clean line
  separation ([4b348e8](https://github.com/maboloshi/hok/commit/4b348e8))
* **build:** Move hok-shim embedding to `libscoop/build.rs`, fix release
  build ([2bbfe85](https://github.com/maboloshi/hok/commit/2bbfe85))

### Performance

* **release:** Optimize release build with rust-lld + clang-cl for thinner
  binaries ([acc7aaa](https://github.com/maboloshi/hok/commit/acc7aaa))

### CI

* **release:** Restore release workflow — manual dispatch + build + upload
  ([8ea27bb](https://github.com/maboloshi/hok/commit/8ea27bb))

## [0.2.0-beta.1](https://github.com/maboloshi/hok/compare/v0.2.0-alpha.2...v0.2.0-beta.1) (2026-07-26)

### ⚠ BREAKING CHANGES

* **i18n:** All user-facing messages migrated to `rust_i18n::t!()`. Third-party
  tools parsing CLI output should use `--quiet` or machine-readable flags.
  Custom language packs can now be added via `locales/{locale}.yml`.
* **output:** Output style config key `output-style` replaces `pacman-style`.
  Old config values are auto-migrated on first use.
* **list:** Table format replaces plain list — column widths now dynamic.
  `hok list -k` output columns are realigned.

### Features

* **hok-shim:** no_std native Shim launcher (10 KB, zero deps, full spec compliance)
  * PE header parsing for GUI/console detection (no `shell32` dependency)
  * `AttachConsole` for console targets (avoids ~400ms console allocation)
  * `CreateJobObject` + `KILL_ON_JOB_CLOSE` for child cleanup
  * `SetConsoleCtrlHandler` with real handler (not NULL, per MSDN spec)
  * `ShellExecuteExW` for elevation — waits + forwards exit code
  * `%~dp0` and `%ENV%` expansion, `~\\..\\` relative path resolution
  * Benchmark: +22ms overhead over direct execution
  * Embedded into `hok.exe` at compile time — no separate build step
* **i18n:** Full user-facing message internationalization framework
  * `locales/en.yml` + `locales/zh.yml` — switch via `LANG` env or config
  * All 28 commands + eventloop migrated to `t!()` keys
  * `hok-i18n-derive` proc-macro for CLI `--help`/`-h` i18n
  * `about`/`long_about` split — examples only in `--help` (not `-h`)
* **output:** Switchable style — `hok config set output-style pacman`
  * Scoop style (default): step-by-step progress messages
  * Pacman style: `::` headers, `✓`/`⚠`/`✗` icons, bold tags, plain content
  * `--detail` global flag for verbose extraction progress (Scoop style always shows steps)
* **list:** Table format with I18n headers, CJK-aware column alignment
  * `-u` flag filters to upgradable packages; `Info` column always shows `→ version`
  * `-k` output columns realigned
* **cleanup:** `*` wildcard and `--all/-a` flag support
  * Fail count with retry hint on partial failures
  * i18n for `--all`/`--global` args
* **install:** `--global/-g` flag for install, uninstall, cleanup, hold, unhold
* **installer.file:** Support `installer.file` + `installer.args` manifest format
  * ShellExecuteExW for proper GUI installation window display
  * Scoop variable expansion in args (`$dir`, `$scoopdir`, `$version`, etc.)
  * URL fragment rename (`#/installer.exe` → direct copy)
  * No hash-named duplicates in working directory
* **uninstall:** `SyncOption::Remove` now properly applied — no more full-bucket scans
* **reinstall:** Bucket-qualified queries, `--assume-yes` respected for uninstall, auto-confirm install
* **PS preamble:** Complete Scoop-compatible variables: `$bucketsdir`, `$bucket`, `$cmd`
* **notes:** Manifest `notes` field displayed after package install
* **bucket list:** Shows HEAD commit author date (`2026/7/25 17:00:56` format)
* **version:** Build timestamp in `hok --version` output
* **held:** Upgrade summary shows held package warnings
* **extract:** Inline progress bars for both internal (sevenz-rust2/zip) and external (7z.exe) paths
* **git:** `reset_head` now properly updates working tree after fetch (fast-forward check, no silent skip)

### Fixes

* **hok-shim:** `SetConsoleCtrlHandler(NULL, TRUE)` → real handler function (MSDN compliance)
* **hok-shim:** `ShellExecuteW` → `ShellExecuteExW` (elevation waits + forwards exit code)
* **hok-shim:** `expand_dp0` wrapping_sub panic on empty args in .shim files
* **sort:** Case-insensitive sorting for packages, buckets, and candidates
* **output:** `-h` no longer shows smashed-together examples in `about` field
* **i18n:** Shim command arg help text correctly translated (was in SKIP list)
* **i18n:** Duplicate `args` key in shim YAML removed, `name` translation restored
* **last_update:** Timestamp format aligned with Scoop (`[DateTime]::Now.ToString('o')`)
* **export:** All installed packages and bucket URLs are now correctly exported
* **transaction:** Removed packages shown in confirm prompt
* **cancel:** "All up to date" message suppressed when user cancels transaction
* **cleanup:** Failure count only counts successful removals; retry hint shown
* **test:** Installer.file execution test uses `cmd.exe /c` and PowerShell (no direct exe assumptions)

### Performance

* **sync:** Resolve and install no longer perform full-bucket scans for exact queries
* **deps:** `hok-i18n-derive` as workspace member (minimal overhead at build time)

### Tests

* hok-shim: 36 unit tests for parser functions, path resolution, env expansion
* libscoop: 65 unit tests (installer.file, URL fragment, variable expansion, etc.)
* Total: 101 tests, all passing

### Features

* **output:** unified output system with pacman-style coloring (`output::*` helpers)
* **output:** `--detail` global flag for verbose per-package progress visibility
* **shim:** native Rust shim launcher (`hok-shim.exe`) with fallback to `.cmd` wrapper
* **shortcut:** pure Rust `.lnk` writer with args/icon support — no COM FFI
* **reinstall:** new command — uninstall + same-version reinstall with held-state preservation
* **update:** 15-minute cooldown to prevent short-term repeated bucket updates
* **update:** `--force` flag to bypass cooldown
* **update:** visible manifest cache refresh progress

### Performance

* **update:** fetch only current HEAD branch instead of all branches
* **libscoop:** optimized package queries — skip full scans for exact lookups

### Fixes

* **shortcut:** replaced unreliable IShellLinkW COM FFI with `shortcuts-rs` crate
* **manifest:** preserve JSON formatting when updating hashes/versions (text patching)
* **install:** aligned commit order with Scoop (pre_install → extract → link_current → post_install)
* **persist:** directory → junction, file → hard link
* **cooldown:** skip `hok update` cooldown when `--offline` is active
* **install:** `pre_install`/`post_install` scripts now execute correctly (was silently skipped)

### Tests

* integration test fixtures with Scoop-original compatibility tracking
* manifest parsing tests (simple, architecture, checkver, dependencies, script blocks)
* shortcut creation tests (basic, with args/icon)
* version comparison tests (9 edge case scenarios)

> **Fork Notice**: This release is a community-maintained fork based on the original
> v0.1.0-beta.7. Core dependencies have been rewritten (HTTP, datetime, hash backend),
> so this fork starts a new alpha cycle for stability verification.

### ⚠ BREAKING CHANGES

* **libscoop:** `curl` HTTP backend replaced with `ureq` (pure Rust). Proxy config
  continues to work, but custom curl options are no longer supported.
* **libscoop:** `sysinfo` crate replaced with raw Win32 FFI for process enumeration
* **libscoop:** `chrono` dependency replaced with `jiff`. Config timestamps use
  ISO 8601 format instead of RFC 3339 with microseconds.
* **libscoop:** removed `rustcrypto-hash` feature flag — `scoop_hash` now defaults
  to `rustcrypto` backend (RustCrypto crates). Self-contained backend removed.
* **libscoop:** `Error::Curl` and `Error::CurlMulti` variants removed.
* **libscoop:** `once_cell` removed — uses std `LazyLock`/`OnceCell`/`OnceLock`
  (requires Rust 1.80+).

### Features

* **checkver:** complete implementation with all extraction modes:
  * regex, JSONPath, XPath, PowerShell script
  * reverse (last match), replace (capture group templates)
  * GitHub shortcut (API `$.tag_name`)
  * SourceForge shortcut (RSS feed)
* **checkver:** autoupdate `--update` with full manifest rewriting:
  * `$version`, `$matchN`, `$matchHead`, `$matchTail`, `$basename` variables
  * Hash extraction from remote page (jsonpath > regex > find)
  * Per-architecture URL/hash handling (32bit/64bit/arm64)
* **libscoop:** SQLite manifest cache (`use_sqlite_cache` config)
  * Compatible with original Scoop's schema at `{cache}/scoop.db`
  * Auto-populated on `hok update` and on first query
* **config:** `ignore_failures` setting (install/upgrade/uninstall continue on error)
* **hok:** new commands: `depends`, `prefix`, `which`, `checkup`, `alias`,
  `export`, `import`, `create`, `shim`, `virustotal`
* **hok:** `cleanup` command implementation (removes old package versions)
* **hok:** `update` command now accepts package names (Scoop-compatible)
* **hok:** `checkhashes` streaming download + hash in single pass

### Performance Improvements

* **regex:** reduced feature set (dropped unicode-perl/bool/gencat). Compile
  time reduced ~47% (5:21 → 2:48). Binary size unchanged (LTO already optimized).
* **libscoop:** `futures` thread-pool replaced with `std::thread::spawn`.
  Bucket update parallelism unchanged, no async runtime overhead.
* **libscoop:** `once_cell` → std equivalents (smaller dep tree, faster compile)
* **libscoop:** `compare_versions` rewritten — proper text segment, pre-release,
  and v-prefix handling

### Code Refactoring

* **libscoop:** `chrono` → `jiff` (lighter datetime crate, same author as `regex`)
* **libscoop:** `curl` + `static-curl` → `ureq` (pure Rust HTTP, no C build deps)
* **libscoop:** `sysinfo` → raw Win32 FFI for `running_apps()`
* **libscoop:** `scoop_hash` selfcontained backend removed (use RustCrypto)
* **libscoop:** old curl/git2/sysinfo/chrono code kept as comments for reference
* **libscoop:** removed unix-only `openssl` dependency (project is Windows-only)
* **libscoop:** all warnings resolved (0 warnings, 0 errors)

### Features (from original v0.1.0-beta.7)

* **hok:** 27 CLI commands covering all original Scoop functionality
* **libscoop:** Pure Rust archive extraction (7z/zip/tar/gz/bz2/xz/lzh/rar/zst)
* **libscoop:** IShellLinkW COM FFI for shortcuts (zero dependencies)
* **libscoop:** Fragmented download via Range requests (curl/aria2-compatible config)
* **libscoop:** Resumable fragmented downloads — partial parts resume via HTTP Range,
  no restart needed on interruption
* **hok:** reset command with post_install fix (original Scoop bug workaround)
* **hok:** batch failure isolation — `ignore_failures` keeps multi-package operations
  running even if individual packages fail

## [0.1.0-beta.7](https://github.com/chawyehsu/hok/compare/v0.1.0-beta.6...v0.1.0-beta.7) (2024-12-10)


### ⚠ BREAKING CHANGES

* **libscoop:** switch to use `tracing` for logging
* **libscoop:** `manifest.hash()` return type changed from `str` to `HashString`

### Features

* **hok:** add `hok completions` command to generate shell completion ([da1b6d8](https://github.com/chawyehsu/hok/commit/da1b6d8f409d8c7894872dab84e28cb8d1814fab))
* **hok:** add global `--verbose` flag ([5fd0505](https://github.com/chawyehsu/hok/commit/5fd050584e80687452a6dde798824cd312e1b74a))


### Bug Fixes

* **libscoop:** hash checking should be case insensitive (fix [#18](https://github.com/chawyehsu/hok/issues/18)) ([b3afbef](https://github.com/chawyehsu/hok/commit/b3afbef0fd438844af786fa2484fb314d7da0227))


### Code Refactoring

* **libscoop:** switch to use `tracing` for logging ([d835c7f](https://github.com/chawyehsu/hok/commit/d835c7fb96db2e99ff1b726bb3e1e5f68b31c2f7))

## [0.1.0-beta.6](https://github.com/chawyehsu/hok/compare/v0.1.0-beta.5...v0.1.0-beta.6) (2024-10-10)


### Features

* **libscoop:** Adpot new cache filename format ([15172a9](https://github.com/chawyehsu/hok/commit/15172a9f7ac35963d1f274e51a4a72de478546c1))


### Bug Fixes

* cargo clippy fix ([71a7596](https://github.com/chawyehsu/hok/commit/71a759605a5ea2b68093cf19f1f3fc8bfe3c15b8))
* **hok:** Check if `cat_style` is empty ([4ed662e](https://github.com/chawyehsu/hok/commit/4ed662ed8bc5d35e3570a8daf1bb6fd92c429f40)), closes [#10](https://github.com/chawyehsu/hok/issues/10)
* **hok:** Remove `type.exe` dep in `hok cat` ([47a42c5](https://github.com/chawyehsu/hok/commit/47a42c57c17276c7024baaf13479b88e4eac81a1))

## [0.1.0-beta.5](https://github.com/chawyehsu/hok/compare/v0.1.0-beta.4...v0.1.0-beta.5) (2024-07-09)


### Features

* **libscoop|config:** support `use_isolated_path` config ([1bb5ee7](https://github.com/chawyehsu/hok/commit/1bb5ee773867c490af8e21885acc87e84a33f40c))
* **libscoop:** remove env paths under isolated_path mode correctly ([2f58173](https://github.com/chawyehsu/hok/commit/2f5817387006a9334f5a74bc7c17d7063e529108))
* **libscoop:** support `use_sqlite_cache` config ([35c9577](https://github.com/chawyehsu/hok/commit/35c9577be0bf497e23c5857e350b7b5717b35645))


### Bug Fixes

* **libscoop|config:** support named `use_isolated_path` ([5e9a181](https://github.com/chawyehsu/hok/commit/5e9a18135ea309248e657c44bcf43a235833a7cf))
* **libscoop:** case insensitive match on package querying ([5efde66](https://github.com/chawyehsu/hok/commit/5efde6661999bd6c7f6bff8d1d9e30d78db5564e))
* **libscoop:** updated Config struct ([17e474a](https://github.com/chawyehsu/hok/commit/17e474a48a03f3c5281af203802a26f79e284e95))

## [0.1.0-beta.4](https://github.com/chawyehsu/hok/compare/v0.1.0-beta.3...v0.1.0-beta.4) (2023-09-09)


### Features

* **hok:** added s shortcut for search command ([50c0bfc](https://github.com/chawyehsu/hok/commit/50c0bfcd6dd928dc105a4ec7afefb1d4e0aa97c7))


### Bug Fixes

* **hok:** added long format arg of listing known buckets ([658ef7d](https://github.com/chawyehsu/hok/commit/658ef7d9e799301bbd5807a195dd2f263933d5c1))
* **hok:** fix 50c0bfc ([387fd66](https://github.com/chawyehsu/hok/commit/387fd66637d7e53d167a18be8a0fc9daf121475e))
* **hok:** trim yes_no prompt input ([0a01f1e](https://github.com/chawyehsu/hok/commit/0a01f1e1ee65e50f9cc5e081d8daa734ea7770e4))
* **libscoop|config:** default config path should be always returned ([d3040ad](https://github.com/chawyehsu/hok/commit/d3040adf732839bb0070f6585be5428ad0d25e73))
* **libscoop|fs:** improve symlink removal logic ([398ef27](https://github.com/chawyehsu/hok/commit/398ef27fc280ded401e0e5fb5a9123d5a165b2af))
* **libscoop|resolve:** correct pinned dependency cascade resolving ([660d3e2](https://github.com/chawyehsu/hok/commit/660d3e2da5bbe5218c45c8706282bfdbc2bfe760))


### Performance Improvements

* **libscoop|manifest:** defer hash validation ([d1ff3f6](https://github.com/chawyehsu/hok/commit/d1ff3f61a46b930771b0d4809fcf77ada2ac04c3))

## [0.1.0-beta.3](https://github.com/chawyehsu/hok/compare/v0.1.0-beta.2...v0.1.0-beta.3) (2023-08-09)


### ⚠ BREAKING CHANGES

* **libscoop|config:** `Package::manifest_path` is replaced by `manifest().path()`.

### Features

* **hok:** reflect basic support of uninstalling packages ([183cfd8](https://github.com/chawyehsu/hok/commit/183cfd8b54e8e96ce2e575240f3b7edb3183f005))
* **libscoop|sync:** basic support of uninstalling packages ([b1f0f6b](https://github.com/chawyehsu/hok/commit/b1f0f6bd3c7ee61b846d60a70889c4033730b10a))


### Bug Fixes

* **hok:** print ending newline for error report ([d1f5682](https://github.com/chawyehsu/hok/commit/d1f56822a1db93cf566265b3ec44082794896422))
* **libscoop|config:** correct `no_junction` field ([4bae700](https://github.com/chawyehsu/hok/commit/4bae700efa06b4d07506370e2eaace04ef747d3d))
* **libscoop|query:** don't create empty apps dir ([7287bd5](https://github.com/chawyehsu/hok/commit/7287bd5672f6eb88ebb52acb928bfbcc6e87877a))
* **libscoop:** added portability on non-windows ([3d1ffee](https://github.com/chawyehsu/hok/commit/3d1ffeeb39074a8c31cbb97891c082fd2a31a7fc))
* **libscoop:** avoid forcing doc target as it will fail to build ([7674f8a](https://github.com/chawyehsu/hok/commit/7674f8aef2f1cdf952c96aed6f16dbb08f65f335))
* **libscoop:** emit BucketUpdateDone event despite zero bucket ([dc4bdca](https://github.com/chawyehsu/hok/commit/dc4bdca39bec812073ca68f332d568e391736ef8))
* **libscoop:** ensure cache dir exist before downloading ([485255e](https://github.com/chawyehsu/hok/commit/485255e926df58efd6e03881d430c8496c9a4adb))
* **scoop-hash:** remove docsrs target ([b6ddd19](https://github.com/chawyehsu/hok/commit/b6ddd19f7c1f70754c70c3b1c6ca87c43e0e0754))

## [0.1.0-beta.2](https://github.com/chawyehsu/hok/compare/v0.1.0-beta.1...v0.1.0-beta.2) (2023-08-03)


### ⚠ BREAKING CHANGES

* **libscoop:** `SyncOption::NoDownloadSize` becomes `SyncOption::Offline`

### Features

* **hok|cat:** show manifest path ([7e06467](https://github.com/chawyehsu/hok/commit/7e064672ebd6aa2009f1db49ea6a0f8704139be3))
* **hok:** show bucket manifest count ([d71e193](https://github.com/chawyehsu/hok/commit/d71e193be2cc20598e53b08947635f67a1409399))
* **libscoop|download:** support injecting cookie defined in manifest ([aec7fdc](https://github.com/chawyehsu/hok/commit/aec7fdc851aee1673170182f7d382a069d514649))
* **libscoop|download:** write to temp file in downloading ([d79e598](https://github.com/chawyehsu/hok/commit/d79e5989aa01b1d49cc02003692e2f4b46991ca0))
* **libscoop|event:** added integrity check event and error type ([888afbb](https://github.com/chawyehsu/hok/commit/888afbba203b80dfd4accf57fbc99dc1b348d3e3))
* **libscoop|manifest:** impl Display for License ([e91ff0e](https://github.com/chawyehsu/hok/commit/e91ff0ec48a295a91d771e7256e542e9cab74846))
* **libscoop|resolve:** allow to select installed candidate ([8fb0ec3](https://github.com/chawyehsu/hok/commit/8fb0ec39509128498be1bcbeb3fcddb5edb16838))
* **libscoop|sync:** added SyncOption::EscapeHold for package remove ([ca8fad7](https://github.com/chawyehsu/hok/commit/ca8fad7ffbd1dd1cb0a6d1e03f924e63c5db3364))
* **libscoop:** added package integrity check logic ([57869f7](https://github.com/chawyehsu/hok/commit/57869f763e5a1a9c3668b3028d46787e5ce0e04d))
* **libscoop:** scoop-hash features passthrough ([cb027ce](https://github.com/chawyehsu/hok/commit/cb027cedd98de15aa17602234b824b240c2fcc2c))
* **scoop-hash:** support switching hashing backend ([d38658e](https://github.com/chawyehsu/hok/commit/d38658ef8785df92189b29df7094dadfc609e14c))
* **scoop-hash:** use builder pattern ([87ca347](https://github.com/chawyehsu/hok/commit/87ca3475bd4d5cb947c4ee2702807f944d92c729))


### Bug Fixes

* **hok|list:** only print upgradable when the flag is used ([558d9d3](https://github.com/chawyehsu/hok/commit/558d9d39603d85657986da43f8c98372ac938e30))
* **hok:** accumulate downloaded bytes properly ([d6fabc8](https://github.com/chawyehsu/hok/commit/d6fabc89aa0bfa1428328bddc248c11fe2e9d8e9))
* **libscoop:** package resolving is infallible when OnlyUpgrade is used ([cabd52b](https://github.com/chawyehsu/hok/commit/cabd52bdb1659bb835ad60d9074b8bbdaf345ad0))
* **libscoop:** set install state for package's upgradable reference ([63a54f7](https://github.com/chawyehsu/hok/commit/63a54f7a36ac1cdcb612437d05c752a97ed9a9e3))
* **libscoop:** use upgradable package reference when available ([9dfd93f](https://github.com/chawyehsu/hok/commit/9dfd93fcf58980a113bec1eb781f414c30489de9))


### Performance Improvements

* **libscoop:** 5x speedup on package querying ([90a8815](https://github.com/chawyehsu/hok/commit/90a881550df4c3196cd185ab34e4621f854a41b7))

## [0.1.0-beta.1](https://github.com/chawyehsu/hok/compare/v0.1.0-alpha.3...v0.1.0-beta.1) (2023-07-30)


### ⚠ BREAKING CHANGES

* **libscoop:** Some `Event` variants related to bucekt update progress have been updated to fit the latest codebase.

### Features

* **hok:** support resolving and downloading packages ([bdc08dd](https://github.com/chawyehsu/hok/commit/bdc08dd63898f7af22fa538f20b3fb068e87c26f))
* **libscoop|config:** support `SCOOP_CACHE` and `SCOOP_GLOBAL` envs ([cf2a2a5](https://github.com/chawyehsu/hok/commit/cf2a2a5503c93e5d57b5ac72aec490e2d53b2a7d))
* **libscoop|resolve:** added `resolve_cascade` ([0aa0c52](https://github.com/chawyehsu/hok/commit/0aa0c52802ea2238a31352e9ae0b19c730b7510e))
* **libscoop:** added coordination between `AssumeYes` and `NoDownloadSize` ([5e9d578](https://github.com/chawyehsu/hok/commit/5e9d5784f62fd0eb64009aa23d6d76847c164f46))
* **libscoop:** added support for package resolution and download ([4ff0d95](https://github.com/chawyehsu/hok/commit/4ff0d9573794c003c440477656e808bd527377a2))
* move to v0.1.0-beta.1 ([e1a2376](https://github.com/chawyehsu/hok/commit/e1a2376e58eb91889d7b102aaa6c415cf7b49ef1))


### Bug Fixes

* **libscoop:** ensure ops working dir exist ([0520ae8](https://github.com/chawyehsu/hok/commit/0520ae8fc6e7e560e343a4dffa4c7b514adf92c3))
* **libscoop:** handle wildcard query in upgrade operation ([639e8c6](https://github.com/chawyehsu/hok/commit/639e8c6680f53c34fbd989fdb60a0ea5e9b92c14))
* **libscoop:** update crate categories metadata ([8d6271d](https://github.com/chawyehsu/hok/commit/8d6271d208c40faf2de32787fe9c5ccf32e303f6))
* **libscoop:** update doc comments ([bcd29b4](https://github.com/chawyehsu/hok/commit/bcd29b4b172f7adf5511de457f13ce74ac676370))

## [0.1.0-alpha.3](https://github.com/chawyehsu/hok/compare/v0.1.0-alpha.2...v0.1.0-alpha.3) (2023-07-25)


### ⚠ BREAKING CHANGES

* **libscoop:** `Session::new()` is now infallible.

### Features

* **hok|config:** config-list shows the path ([679c177](https://github.com/chawyehsu/hok/commit/679c1771c036982941bce62e6db55e9098b4e739))
* **libscoop:** impl Default for Session ([d91177a](https://github.com/chawyehsu/hok/commit/d91177a269698b8fbd7b530f0100da82d4ce8879))
* **libscoop:** support loading config from all possible location ([2bcc649](https://github.com/chawyehsu/hok/commit/2bcc649808e8238bef5795c73eab41c182cac61b))
* move to v0.1.0-alpha.3 ([1ecd0ed](https://github.com/chawyehsu/hok/commit/1ecd0edf100ea4a3676494b40b5c72c787ad5501))


### Bug Fixes

* **ci:** remove unneeded condition ([718084f](https://github.com/chawyehsu/hok/commit/718084f80c615513c69a838205e58edd2a553d44))
* **libscoop|fs:** `write_json` should create file instead of dir ([54482a7](https://github.com/chawyehsu/hok/commit/54482a7c8c1733e8d0c01bac5e85fc5da7f4fd3e))
* **libscoop:** fix doctest ([c0237a2](https://github.com/chawyehsu/hok/commit/c0237a2e73d976c4f959bb0928da4cbd0ff3376e))

## [0.1.0-alpha.2](https://github.com/chawyehsu/hok/compare/v0.1.0-alpha.1...v0.1.0-alpha.2) (2023-07-25)


### ⚠ BREAKING CHANGES

* **libscoop:** APIs of operations and Session changed.
* **libscoop:** exposed modules of libscoop changed.

### Features

* **hok|cat,home:** support candidate selection ([28b56c5](https://github.com/chawyehsu/hok/commit/28b56c5ade13e1edceb04fa7c0fc7554dcc0c6a9))
* **hok:** add uninstall cmd placeholder ([c13e8be](https://github.com/chawyehsu/hok/commit/c13e8be627ab0bfb91aedfebc10ee89dc2ee8675))
* **hok:** support list held packages ([a2acb22](https://github.com/chawyehsu/hok/commit/a2acb2210bf0586f6d839d61773b1dac7d2f96f1))
* **libscoop|manifest:** support aarch64 specific fields ([639d092](https://github.com/chawyehsu/hok/commit/639d092e22dc32decc98950532614da75489dbe6))
* **libscoop|resolve:** added fn `select_candidate` ([0e296ea](https://github.com/chawyehsu/hok/commit/0e296ea5b0cb2ab884c74ccea42df86ca05840e0))
* **libscoop:** add package resolving and event bus ([434eebe](https://github.com/chawyehsu/hok/commit/434eebe3d464edb48a1d034d4e746810ba41d274))
* **libscoop:** replace ureq with libcurl ([7d3df7c](https://github.com/chawyehsu/hok/commit/7d3df7c3e954187318d46958f07d6e4b4ce9fe31))
* move to v0.1.0-alpha.2 ([24e354a](https://github.com/chawyehsu/hok/commit/24e354a7514d74878c550e25457d323e6251ee4b))


### Bug Fixes

* **hok|cat,home:** sort candidates ([c90f3f9](https://github.com/chawyehsu/hok/commit/c90f3f94367dae75cabd2dd0a562f38c924f6dbd))
* **libscoop:** dag should check self cyclic ([a5bbb0b](https://github.com/chawyehsu/hok/commit/a5bbb0bb5f57ef6d8d326e6eca9bb828f6ff6ec9))


### Miscellaneous Chores

* **libscoop:** tweak exposed modules ([f31cb64](https://github.com/chawyehsu/hok/commit/f31cb64d3794edf01b55757bb3ecdc19d4878932))

## 0.1.0-alpha.1 (2023-07-21)


### Features

* add hash crate ([aa021fb](https://github.com/chawyehsu/hok/commit/aa021fb7fa6eaa3167f803608982307ebbafe9f7))
* **api:** Introduce SPDX spec for manifest.license ([ec5e1f5](https://github.com/chawyehsu/hok/commit/ec5e1f5c6286100724f346ab55ab7fc11d02d5fe))
* **cache:** implement cache-rm ([869f095](https://github.com/chawyehsu/hok/commit/869f0956a0ccb6a8dc06d40d95bde9f79b09e504))
* **cmd:** Implement cleanup, refactor cache and list ([3ca3f26](https://github.com/chawyehsu/hok/commit/3ca3f2610ec5bf0164bfde1d4f91484423cc78c4))
* **cmd:** Implement scoop list subcommand ([0b2fdec](https://github.com/chawyehsu/hok/commit/0b2fdec835835b68b500a19d450f39e82c08a4b6))
* **cmd:** prototype of scoop home ([29c2663](https://github.com/chawyehsu/hok/commit/29c2663768e7bed616e104c6a5339b55bcdf7536))
* **cmd:** prototype of scoop info ([a207465](https://github.com/chawyehsu/hok/commit/a207465b73a704ef31014ccd408c323c45cbbdb5))
* **cmd:** prototype of scoop search (local) ([2c8e563](https://github.com/chawyehsu/hok/commit/2c8e563748539b63e6c95b9c09dbe9b1b1995199))
* **core:** add DepGraph implementation ([ca5f49f](https://github.com/chawyehsu/hok/commit/ca5f49fcd23437a5d257fd83fd23cf1c512cdb27))
* **core:** Implement update subcommand ([ad04e76](https://github.com/chawyehsu/hok/commit/ad04e76762de55954d070be3a3a352b29a78981e))
* **hash-md5:** add reset api ([3db7116](https://github.com/chawyehsu/hok/commit/3db7116412729ff2ca84de93ecd1a1850e17100e))
* **hash:** add checksum helper functions ([df24980](https://github.com/chawyehsu/hok/commit/df24980c664699b24a2efc7b609c2ba324521333))
* **hash:** add sha1 implementation ([8bee89a](https://github.com/chawyehsu/hok/commit/8bee89ae49f30cfbdb42c76c52331c7fd5ba8b82))
* **hash:** add sha256 implementation ([37f9f62](https://github.com/chawyehsu/hok/commit/37f9f622e79a5ec4d3bf122ccafd191d46041c2b))
* **hash:** add sha512 implementation ([7bbecf1](https://github.com/chawyehsu/hok/commit/7bbecf1310ee342e4d4376e413f852b16f6aadd2))
* **hash:** provided a top-level checksum api ([99fed09](https://github.com/chawyehsu/hok/commit/99fed093d48d5cf91f3db0f46f02c4d152d17043))
* Implement basic file downloads ([c5d303b](https://github.com/chawyehsu/hok/commit/c5d303bff23993ca4bc53946c074058a542a0420))
* implement hold and unhold ([682c63c](https://github.com/chawyehsu/hok/commit/682c63c78390ee4300a6c9ad42934b79be7b5866))
* implement status ([bb650d6](https://github.com/chawyehsu/hok/commit/bb650d64c711f74ff1f73c3026b86c90daafe14b))
* **scoop-cache:** implement scoop cache show ([ae018b8](https://github.com/chawyehsu/hok/commit/ae018b86a3abfe23d4f6f9c17edc9047947af8e4))
* **scoop-cache:** implement scoop cache show ([c584c90](https://github.com/chawyehsu/hok/commit/c584c90ff3e90e8744841ea64e3f732a29571b55))
* **scoop-config:** implement scoop config ([9bdc9fa](https://github.com/chawyehsu/hok/commit/9bdc9fa8a46897dea3aef636bd92d51a27b7616f))
* **search:** Add fuzzy search option ([53c8998](https://github.com/chawyehsu/hok/commit/53c8998ed98b4a150e19ffb4a10ce7a7e8ab160e))
* **search:** Implement binary search ([26ff6f2](https://github.com/chawyehsu/hok/commit/26ff6f248fc323f5be1e168e10d19fff613e07a9))
* update ([cf505f0](https://github.com/chawyehsu/hok/commit/cf505f0e51ac4c5777e260651d0ee0cd5e805abb))
* v0.1.0-alpha.1 ([f304bb2](https://github.com/chawyehsu/hok/commit/f304bb262dc1f850ae3932bb810ab91ee272fd2b))


### Bug Fixes

* **bucket-rm:** use remove_dir_all crate ([a5c9a0b](https://github.com/chawyehsu/hok/commit/a5c9a0bb309a54bae2e80552d4a5c9c0b5a4ef16))
* **core:** fix cache regex ([98f2a44](https://github.com/chawyehsu/hok/commit/98f2a44d872c876c6925e6a5ffadbc4864ddfb71))
* **core:** fix manifest download urls extraction ([7fef94c](https://github.com/chawyehsu/hok/commit/7fef94cb1235ce446d10bf0ab09bc853fc1ccd0e))
* Fix apps_in_local_bucket ([b3170c7](https://github.com/chawyehsu/hok/commit/b3170c72263dabacb027e8b66b1be3ce7113bfb7))
* fix cache rm handler ([22f51e2](https://github.com/chawyehsu/hok/commit/22f51e2cb82a80d1123895452ed2cecbf0d09b4a))
* Fix not truncating previous data when saving new configs ([ea4bf0c](https://github.com/chawyehsu/hok/commit/ea4bf0c1fa28d7ede20d35cedc5423c515a4a029))
* typo ([4ddc72f](https://github.com/chawyehsu/hok/commit/4ddc72f944d1fa235fd9644e9ec7896cf917ccc3))
* use init method to create config instance ([9aa08ba](https://github.com/chawyehsu/hok/commit/9aa08ba9caedefb62b86c4c70593463aacaeefae))


### Performance Improvements

* don't update install info if it's not held ([d8b71b1](https://github.com/chawyehsu/hok/commit/d8b71b117d97380085b14c145a59495e0ccae5f3))
* **hash-md5:** use inline fn for performance ([7f35660](https://github.com/chawyehsu/hok/commit/7f356602a93091da5b0017de3a0cb00b3a0e1bb4))
* search enhancement ([13075cc](https://github.com/chawyehsu/hok/commit/13075cc11a267d98296541fdaf3582b2f9f50eca))
