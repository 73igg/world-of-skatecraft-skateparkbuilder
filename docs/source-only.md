# Source-only publishing

This repository distributes the mod's source code, authored interface and shaders, build and
conversion tools, source patches for external open-source engines, documentation, and licenses.
The owner's README showcase videos and abilities screenshot are explicitly approved documentation.
It does not distribute World of Warcraft or Skate game files or assets. Every player supplies
their own game copies and converts any required assets locally.

Never commit, push, attach to an issue, or upload to GitHub Releases:

- Game installations, archives, executables, models, textures, animations, audio, maps, DBCs,
  extracted server data, or converted game content, including JSON model/rig/catalog exports.
- `WoW`, `skate-data/`, `skate-audio/`, `server/`, `benilla-config/`, `.setup/`, or `target/`.
- Server databases, player saves, account credentials, environment files, logs, or binary builds.

The converter code is publishable; its output stays on the player's machine. The Park Builder's
catalog and preview assets are generated into `benilla-config/park-assets/` from the owner's
copies. Custom server and native-engine changes are distributed as text patches under `setup/`.
Their downloaded source checkouts, build contexts and Docker images remain local. Preserve all
upstream licenses and the credits in [NOTICE](../NOTICE).

Fresh installs use vmangos-deploy's normal database initialization and create a new user-chosen
account. Do not export the developer's running databases. Mod table definitions and migration
logic may be published as source; character/account rows, passwords, saved preferences, flags,
session objects and all other personal server state must remain local.

Before committing, stage explicit source paths and run:

```sh
python3 scripts/audit_source.py --index
```

On Windows, use `python` instead of `python3`. Enable the checked-in push guard once per clone:

```sh
git config core.hooksPath .githooks
```

The audit reads Git's staged or committed blobs, rejects paths outside the source allowlist,
unapproved binary and non-UTF-8 content, binary patch payloads, oversized source files, links, submodules and
common credential patterns. The push guard inspects every new commit, including files added
and later deleted. GitHub Actions runs the same check. `.gitignore` also excludes local game
and generated content; forcing a file into the index does not bypass the audit. Review any
source allowlist change carefully: a text extension alone does not establish provenance.

The GitHub fork retains Kimmo3223's upstream history. That history contains the upstream
README's gameplay recording and an older benilla banner, which are documentation media rather
than redistributed game resource files. This fork removes the recordings from its current
source tree and adds no media or game assets. New commits must contain source and documentation
only, plus the owner's approved README showcase media. The audit permits those specific media
paths only when their content matches the approved SHA-256 hashes. Their video previews are
frames from the same recordings. Replacing documentation media requires reviewing and updating
the exact file hashes; adding other binary assets remains blocked. The audit starts at the
inherited upstream revision.

Use `python3 scripts/format_workspace.py --check` to check the Rust workspace. It selects all
workspace members explicitly so Cargo does not also format the downloaded external engine.
That upstream checkout declares test modules it does not distribute; it is not source owned
by this fork, and its paths and contents must stay under the ignored `.setup/` directory.
