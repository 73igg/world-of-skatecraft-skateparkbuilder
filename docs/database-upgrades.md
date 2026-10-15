# Keeping your characters when switching to this fork

Use your existing database. The mod's spells are defined by the patched server, and that server
teaches missing abilities at character creation and login. Accounts, character IDs, names,
equipment, money, quests and progress do not need to be replaced. Existing speed/camera settings
and return flags are retained. New characters and old characters get the same abilities.

## Normal setup

Close the game and run the latest setup. It finds the database container belonging to the
`world-of-skatecraft` Docker project, including stopped containers, and reuses that installation's
server folder. The database's volume name and credentials stay the same. Subsequent setup and
play runs remember the selected folder in `.setup/server-path`.

If the old container was removed, or the folder cannot be detected, select it explicitly:

```bat
setup.bat -Server "C:\original-skatecraft\server"
```

```sh
setup/setup.sh --server "/path/to/original-skatecraft/server"
```

Point this at the folder containing your old `compose.yaml`, `config/` and `storage/`, even when
it is outside this fork's checkout. Keep that folder available when playing. If your old server
uses a different Docker project name, set `SKATECRAFT_SERVER_PROJECT` to that same name for both
setup and play. Do not run both installations' servers at once.

Setup builds both Docker images from the same pinned core, deployment source and mod patch.
This avoids combining a current upstream database image with an older patched server. It applies
the deployment's version-1 changes, including root startup with `VMANGOS_UID`/`VMANGOS_GID`,
database credentials in the server environment, and the updated extractor mount. It keeps host
ports, named volumes or database bind mounts, realm settings and other services. The normalized
Compose file is written as JSON, which Docker Compose also accepts as YAML; the original is in
the backup.

Before changing an existing database, setup stops the world/login services, attempts a full
locked SQL dump and stops the database for a complete compressed snapshot of its data volume.
The stopped-volume backup also works if the old container cannot start because of the deployment
version error. Backup failure stops the upgrade before changing Compose or starting the new
database image. Keep the original Docker images until you have verified the upgrade.

The matching database image applies its upstream schema migrations. The mod then creates only
missing preference/flag tables and records its schema version. It does not bulk-insert spell rows
or replace player records. Automatic world-database recreation is disabled to preserve custom
world edits. If upstream reports an edited historical migration, setup stops for review instead
of acknowledging it or resetting the database. Customized external database hosts, schema names,
secret-file credentials and container entrypoints need a manual deployment upgrade; setup reports
these before making changes.
If the database already has newer upstream core migrations than this fork's pinned server,
setup refuses to downgrade it. Keep that database; supporting it requires updating the fork's
server source and patch together. Replacing it with an older dump would lose newer progress.

Setup skips account creation when accounts already exist. Log in with your existing username and
password and choose your existing character. Missing abilities appear in the **General** spellbook;
drag them onto your action bars. Your existing action bars are kept.

## Standalone migration and diagnosis

From this fork's folder, with Docker running:

```bat
python setup/migrate_database.py upgrade --server "C:\original-skatecraft\server"
```

On Linux, use `python3`. This performs the backup and database upgrade and leaves the world/login
services stopped. Run normal setup to finish extraction if needed, start and verify the server,
and build the client. For an installation whose assets and client are already built, `play.bat`
or `./play.sh` starts and verifies the upgraded server.

```bat
python setup/migrate_database.py check --live
```

This verifies both configured and running image identities against this checkout's patch, core
and deployment version, checks service health, and checks the database migration. The launchers
also run these checks, so an old base server or a stale patched image produces an upgrade message.
Database checks establish that the correct server is running; logging into the character is what
teaches its missing spells. Keeping a character after a repair alone cannot establish which server
the repair selected.

## Backups and recovery

Backups are private local files under `.setup/database-backups/<timestamp>/` in this fork's
checkout. A complete backup has `manifest.json`, `database-volume.tar.gz`, the old `compose.yaml`
and configuration, and `databases.sql` when the old SQL service was reachable. These contain your
accounts and credentials: keep them outside Git and copy important backups somewhere durable.
Do not delete `.setup/` wholesale or run `docker compose down --volumes` to fix an upgrade.

If you need to return to the pre-upgrade state:

```bat
python setup/migrate_database.py restore --server "C:\original-skatecraft\server" --backup ".setup\database-backups\<timestamp>"
```

The restore verifies the archive, restores it into a **new** Docker volume, stops the selected
services and restores the old Compose configuration and image identities. The current database
volume is retained. It also saves the current Compose file beside the backup. Start the restored
original server with:

```bat
docker compose -p world-of-skatecraft --project-directory "C:\original-skatecraft\server" -f "C:\original-skatecraft\server\compose.yaml" up -d
```

Use your project name if different. A restored base server should be used with the original client,
or upgraded again before playing this fork. Restore returns the database to the backup time;
progress made afterwards stays in the retained previous volume.

## Verification for contributors

```sh
python3 -m unittest discover -s setup -p 'test_*.py' -v
python3 setup/build_server.py
SKATECRAFT_TEST_DATABASE=1 python3 -m unittest discover -s setup -p 'test_database_integration.py' -v
```

The opt-in integration test creates an isolated Docker project with synthetic player records,
custom world data and existing preferences. It checks the upgrade, rerunning it, password
preservation, and recovery into a new volume, then removes only its own test containers/volumes.
