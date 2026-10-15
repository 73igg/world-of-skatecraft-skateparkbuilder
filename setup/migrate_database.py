#!/usr/bin/env python3
"""Upgrade a local Skatecraft deployment in place, keeping its accounts and characters.

Normal setup runs `upgrade`. Use `path --server OLD/server` to select an existing install,
`check --live` to diagnose it, or `upgrade --server OLD/server` to upgrade it directly.
Backups stay private in .setup/database-backups/; no player records ship with the mod.
"""
from __future__ import annotations

import argparse
import copy
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

import build_server

ROOT = Path(__file__).resolve().parents[1]
STATE = ROOT / ".setup"
PROJECT = os.environ.get("SKATECRAFT_SERVER_PROJECT", "world-of-skatecraft")
SCHEMA_VERSION = 1
SERVICES = ("database", "realmd", "mangosd")
SQL_AUTH = 'export MYSQL_PWD="$MARIADB_ROOT_PASSWORD"; exec '
TABLES = {
    "character_skatecraft_speed": "guid INT UNSIGNED NOT NULL PRIMARY KEY, half_steps SMALLINT UNSIGNED NOT NULL DEFAULT 3",
    "character_skatecraft_camera": "guid INT UNSIGNED NOT NULL PRIMARY KEY, half_steps SMALLINT UNSIGNED NOT NULL DEFAULT 8",
    "character_skatecraft_camera_preset": "guid INT UNSIGNED NOT NULL PRIMARY KEY, preset TINYINT UNSIGNED NOT NULL DEFAULT 1",
    "character_skatecraft_flag": "guid INT UNSIGNED NOT NULL PRIMARY KEY, revision BIGINT UNSIGNED NOT NULL, "
        "map INT UNSIGNED NOT NULL, instance INT UNSIGNED NOT NULL, zone INT UNSIGNED NOT NULL, "
        "x DOUBLE NOT NULL, y DOUBLE NOT NULL, z DOUBLE NOT NULL, facing DOUBLE NOT NULL",
}


def run(command: list[str], **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(command, check=True, **kwargs)


def docker_json(*args: str) -> list | dict:
    return json.loads(run(["docker", *args], capture_output=True, text=True).stdout)


def project_containers() -> list[dict]:
    ids = run(["docker", "ps", "-aq", "--filter", f"label=com.docker.compose.project={PROJECT}",
               "--filter", "label=com.docker.compose.service=database"], capture_output=True, text=True).stdout.split()
    return docker_json("inspect", *ids) if ids else []


def select_server(explicit: Path | None = None) -> Path:
    if explicit:
        return explicit.expanduser().resolve()
    saved = STATE / "server-path"
    if saved.exists():
        return Path(saved.read_text(encoding="utf-8-sig").strip()).resolve()
    containers = project_containers()
    if len(containers) > 1:
        raise ValueError("Multiple database containers found. Specify --server and SKATECRAFT_SERVER_PROJECT.")
    if containers:
        labels = containers[0]["Config"].get("Labels") or {}
        old = Path(labels.get("com.docker.compose.project.working_dir", ""))
        if old.is_absolute() and (old / "compose.yaml").is_file():
            return old.resolve()
        raise ValueError("An existing database was found, but its server folder is unavailable. "
                         "Pass --server pointing to that installation's server folder.")
    return ROOT / "server"


def save_server(server: Path) -> None:
    STATE.mkdir(exist_ok=True)
    (STATE / "server-path").write_text(str(server) + "\n", encoding="utf-8")


class Deployment:
    def __init__(self, server: Path):
        self.server = server
        self.compose = server / "compose.yaml"
        self.command = ["docker", "compose", "-p", PROJECT, "--project-directory", str(server), "-f", str(self.compose)]

    def call(self, *args: str, **kwargs) -> subprocess.CompletedProcess:
        return run(self.command + list(args), **kwargs)

    def config(self) -> dict:
        # Compose escapes every dollar sign in its rendered output for reuse as a Compose file.
        output = self.call("config", "--format", "json", capture_output=True, text=True).stdout
        return json.loads(output.replace("$$", "$"))

    def sql(self, query: str) -> str:
        # The password is read inside the container, never put on a host command line or in logs.
        return self.call("exec", "-T", "database", "sh", "-c", SQL_AUTH + "mariadb -uroot -NB",
                         input=query, capture_output=True, text=True).stdout.strip()


def db_mount(config: dict) -> dict:
    matches = [v for v in config["services"]["database"].get("volumes", []) if v["target"] == "/var/lib/mysql"]
    if len(matches) != 1 or matches[0]["type"] not in ("bind", "volume"):
        raise ValueError("Expected one persistent database mount at /var/lib/mysql; refusing to create a replacement.")
    mount = copy.deepcopy(matches[0])
    if mount["type"] == "volume":
        mount["source"] = config["volumes"][mount["source"]]["name"]
    return mount


def mount_args(mount: dict, target: str, readonly: bool = False) -> list[str]:
    # Docker's --mount parser supports quoted CSV fields, including paths containing commas.
    import csv
    import io
    row = io.StringIO()
    csv.writer(row, lineterminator="").writerow([
        f"type={mount['type']}", f"source={mount['source']}", f"target={target}",
        *(["readonly"] if readonly else []),
    ])
    return ["--mount", row.getvalue()]


def canonical_mount(source: str) -> str:
    source = source.replace("\\", "/").rstrip("/")
    prefix = "/run/desktop/mnt/host/"
    if source.startswith(prefix):
        source = source[len(prefix)] + ":" + source[len(prefix) + 1:]
    if len(source) > 1 and source[1] == ":":
        source = source.casefold()
    return source


def validate_existing(config: dict, containers: list[dict]) -> dict | None:
    if len(containers) > 1:
        raise ValueError("Multiple database containers found; select their project explicitly.")
    mount = db_mount(config)
    if not containers:
        return None
    container = containers[0]
    actual = [v for v in container["Mounts"] if v["Destination"] == "/var/lib/mysql"]
    if len(actual) != 1:
        raise ValueError("The existing database has an unrecognized data mount.")
    actual_source = actual[0].get("Name") if actual[0]["Type"] == "volume" else actual[0]["Source"]
    if mount["type"] != actual[0]["Type"] or canonical_mount(mount["source"]) != canonical_mount(actual_source):
        raise ValueError("This Compose file points to different storage than the existing database. "
                         "Use --server with the original installation; no database was changed.")
    return container


def normalize_config(config: dict, images: dict[str, str]) -> dict:
    result = copy.deepcopy(config)
    services = result["services"]
    if any(name not in services for name in SERVICES):
        raise ValueError("Expected database, realmd and mangosd services in compose.yaml.")
    db_mount(result)
    env = services["database"].setdefault("environment", {})
    if any(key.endswith("_FILE") for key in env if "PASSWORD" in key or "USER" in key):
        raise ValueError("Secret-file database credentials require a manual deployment upgrade; no data was changed.")
    for key, default in (("MARIADB_USER", "mangos"), ("MARIADB_PASSWORD", "mangos"), ("MARIADB_ROOT_PASSWORD", "password")):
        env.setdefault(key, default)
        if not env[key]:
            raise ValueError(f"{key} is empty; keep the credentials of the existing database.")
    # A migration must never trigger the deploy image's destructive world-database correction path.
    env["VMANGOS_ENABLE_AUTOMATIC_WORLD_DB_CORRECTIONS"] = "0"
    env["VMANGOS_HALT_ON_MIGRATION_EDITS"] = "1"
    services["database"]["stop_grace_period"] = "2m"
    for name in SERVICES:
        service = services[name]
        expected_command = {"database": "mariadbd", "realmd": "realmd", "mangosd": "mangosd"}[name]
        if service.get("command") not in (None, [expected_command], expected_command):
            raise ValueError(f"Custom command on {name}; review it before upgrading.")
        current = service.setdefault("environment", {})
        version = str(current.get("VMANGOS_DEPLOY_VERSION", "0"))
        if version not in ("0", "1"):
            raise ValueError("This deployment is newer than the supported version; refusing a downgrade.")
        current["VMANGOS_DEPLOY_VERSION"] = build_server.DEPLOY_VERSION
        service["image"] = images["database" if name == "database" else "server"]
        # Compose build/entrypoint overrides could silently start an unpatched server.
        if service.get("build") or service.get("entrypoint"):
            raise ValueError(f"Custom build/entrypoint on {name}; upgrade that customization explicitly.")
        service["pull_policy"] = "never"
        if name != "database":
            uid, gid = "1000", "1000"
            user = str(service.pop("user", ""))
            if user:
                parts = user.split(":")
                if len(parts) != 2 or not all(part.isdigit() and int(part) > 0 for part in parts):
                    raise ValueError("Expected a numeric non-root UID:GID; review the server's user configuration.")
                uid, gid = parts
            current.setdefault("VMANGOS_UID", uid)
            current.setdefault("VMANGOS_GID", gid)
            current["VMANGOS_DATABASE_USER"] = env["MARIADB_USER"]
            current["VMANGOS_DATABASE_PASSWORD"] = env["MARIADB_PASSWORD"]
    # Tool services from version 1 must use the same patched image and environment.
    for name in ("extract-client-data", "check-deploy-version"):
        if name in services:
            services[name]["image"] = images["server"]
            services[name]["environment"] = copy.deepcopy(services["mangosd"]["environment"])
            services[name].pop("user", None)
            services[name]["pull_policy"] = "never"
    return result


def validate_server_settings(config: dict) -> None:
    expected = {"LoginDatabaseInfo": "realmd", "LoginDatabase.Info": "realmd",
                "WorldDatabase.Info": "mangos", "CharacterDatabase.Info": "characters", "LogsDatabase.Info": "logs"}
    for name in ("realmd", "mangosd"):
        target = f"/opt/vmangos/config/{name}.conf"
        mounts = [v for v in config["services"][name].get("volumes", []) if v["target"] == target]
        if len(mounts) != 1 or mounts[0]["type"] != "bind" or not Path(mounts[0]["source"]).is_file():
            raise ValueError(f"The {name} configuration file is missing or uses an unsupported mount.")
        content = Path(mounts[0]["source"]).read_text(encoding="utf-8-sig")
        for key, schema in expected.items():
            match = re.search(r'(?m)^\s*' + re.escape(key) + r'\s*=\s*"([^"\r\n]+)"', content)
            if match:
                parts = match[1].split(";")
                if len(parts) != 5 or parts[0] != "database" or parts[4] != schema:
                    raise ValueError("Custom database hosts/schema names require a manual migration. "
                                     "This script upgrades the local database service only.")
        defaults = {"mangosd": (8085, 7878, 3443), "realmd": (3724,)}[name]
        if any(int(port["target"]) not in defaults for port in config["services"][name].get("ports", [])):
            raise ValueError("Custom container ports require a manual deployment upgrade. Host port mappings are supported.")


def escape_compose(value):
    # `compose config` resolves interpolation. Escape literal dollar signs on serialization.
    if isinstance(value, str):
        return value.replace("$", "$$")
    if isinstance(value, dict):
        return {key: escape_compose(item) for key, item in value.items()}
    if isinstance(value, list):
        return [escape_compose(item) for item in value]
    return value


def write_config(path: Path, config: dict) -> None:
    # JSON is valid YAML and avoids adding a Python YAML dependency to setup.
    temporary = path.with_suffix(".yaml.pending")
    temporary.write_text(json.dumps(escape_compose(config), indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def backup(deploy: Deployment, config: dict, container: dict | None) -> Path | None:
    mount = db_mount(config)
    users = run(["docker", "ps", "-q", "--no-trunc", "--filter", f"volume={mount['source']}"],
                capture_output=True, text=True).stdout.split()
    if any(item != (container or {}).get("Id") for item in users):
        raise ValueError("This database storage is in use by another container. Select the original Docker project before upgrading.")
    if mount["type"] == "volume":
        found = subprocess.run(["docker", "volume", "inspect", mount["source"]], capture_output=True)
        if found.returncode and container is None:
            if config["volumes"][next(v["source"] for v in config["services"]["database"]["volumes"]
                                     if v["target"] == "/var/lib/mysql")].get("external"):
                raise ValueError("The external database volume is missing; refusing to initialize an empty replacement.")
            return None
        if found.returncode:
            raise ValueError("The existing database volume is missing.")
    elif not Path(mount["source"]).exists():
        if container:
            raise ValueError("The existing database folder is missing.")
        return None
    image = container["Image"] if container else config["services"]["database"]["image"]
    if subprocess.run(["docker", "image", "inspect", image], capture_output=True).returncode:
        run(["docker", "pull", image])
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    destination = STATE / "database-backups" / stamp
    destination.mkdir(parents=True)
    shutil.copy2(deploy.compose, destination / "compose.yaml")
    for name in ("config", ".env"):
        source = deploy.server / name
        if source.is_dir():
            shutil.copytree(source, destination / name)
        elif source.is_file():
            shutil.copy2(source, destination / name)
    print(f"Saving database backup to {destination}", flush=True)
    # Stop player writes before dumping: vmangos has MyISAM tables, so a single transaction is insufficient.
    deploy.call("stop", "mangosd", "realmd")
    if container and container["State"]["Running"]:
        try:
            with (destination / "databases.sql").open("wb") as output:
                run(["docker", "exec", container["Id"], "sh", "-c", SQL_AUTH +
                     "mariadb-dump -uroot --all-databases --lock-all-tables --routines --events --triggers --hex-blob"],
                    stdout=output, stderr=subprocess.PIPE)
        except subprocess.CalledProcessError:
            (destination / "databases.sql").unlink(missing_ok=True)
            print("The old SQL service is unavailable; preserving its complete stopped data volume instead.")
    if container:
        run(["docker", "stop", "--time", "120", container["Id"]], stdout=subprocess.DEVNULL)
        stopped = docker_json("inspect", container["Id"])[0]["State"]
        if stopped.get("OOMKilled") or stopped.get("ExitCode") == 137:
            raise ValueError("The database did not stop cleanly. Restart the original database and retry before upgrading.")
    command = ["docker", "run", "--rm", "--network", "none", "--user", "0:0", "--entrypoint", "tar"]
    command += mount_args(mount, "/source", readonly=True)
    command += mount_args({"type": "bind", "source": str(destination)}, "/backup")
    run(command + [image, "-czf", "/backup/database-volume.tar.gz", "-C", "/source", "."])
    run(command + [image, "-tzf", "/backup/database-volume.tar.gz"], stdout=subprocess.DEVNULL)
    old_images = {}
    for name in SERVICES:
        ids = deploy.call("ps", "-aq", name, capture_output=True, text=True).stdout.split()
        if len(ids) == 1:
            old_images[name] = docker_json("inspect", ids[0])[0]["Image"]
            continue
        old_image = config["services"][name]["image"]
        found = subprocess.run(["docker", "image", "inspect", old_image], capture_output=True, text=True)
        if found.returncode == 0:
            old_images[name] = json.loads(found.stdout)[0]["Id"]
    old_images["database"] = image
    (destination / "manifest.json").write_text(json.dumps({
        "complete": True, "project": PROJECT, "server": str(deploy.server), "mount": mount,
        "database_image": image, "old_images": old_images, "old_config": config,
    }, indent=2), encoding="utf-8")
    return destination


def restore(deploy: Deployment, backup_path: Path) -> None:
    backup_path = backup_path.resolve()
    manifest = json.loads((backup_path / "manifest.json").read_text(encoding="utf-8"))
    if not manifest.get("complete") or manifest["project"] != PROJECT or Path(manifest["server"]).resolve() != deploy.server:
        raise ValueError("This is not a complete backup for the selected server and Docker project.")
    image = manifest["database_image"]
    run(["docker", "image", "inspect", image], stdout=subprocess.DEVNULL)
    command = ["docker", "run", "--rm", "--network", "none", "--user", "0:0", "--entrypoint", "tar"]
    command += mount_args({"type": "bind", "source": str(backup_path)}, "/backup", readonly=True)
    run(command + [image, "-tzf", "/backup/database-volume.tar.gz"], stdout=subprocess.DEVNULL)
    volume = PROJECT + "-restored-" + datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S%f")
    run(["docker", "volume", "create", volume], stdout=subprocess.DEVNULL)
    command += mount_args({"type": "volume", "source": volume}, "/restore")
    run(command + [image, "-xzf", "/backup/database-volume.tar.gz", "-C", "/restore"])
    original = manifest["old_config"]
    for name, image_id in manifest.get("old_images", {}).items():
        original["services"][name]["image"] = image_id
        original["services"][name]["pull_policy"] = "never"
    volumes = original["services"]["database"]["volumes"]
    volumes[:] = [v for v in volumes if v["target"] != "/var/lib/mysql"]
    volumes.append({"type": "volume", "source": "skatecraft-restored", "target": "/var/lib/mysql"})
    original.setdefault("volumes", {})["skatecraft-restored"] = {"name": volume, "external": True}
    deploy.call("stop", "mangosd", "realmd", "database")
    current = backup_path / ("compose-before-restore-" + datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S%f") + ".yaml")
    shutil.copy2(deploy.compose, current)
    write_config(deploy.compose, original)
    print(f"Backup restored to new volume {volume}. The previous volume is retained. "
          "The original server configuration is restored; start it with Docker Compose up -d.")


def wait_database(deploy: Deployment, timeout: int = 600) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        ids = deploy.call("ps", "-aq", "database", capture_output=True, text=True).stdout.split()
        if ids:
            state = docker_json("inspect", ids[0])[0]["State"]
            if state.get("Health", {}).get("Status") == "healthy":
                return
            if not state.get("Running"):
                break
            pending = subprocess.run(["docker", "exec", ids[0], "test", "-f", "/tmp/vmangos-changes-pending"],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if pending.returncode == 0:
                raise ValueError("Upstream reports an edited migration and requires manual review. Your backup is retained. "
                                 "See the database logs; do not reset the database or blindly confirm the edit.")
        time.sleep(3)
    raise ValueError("The database did not become healthy. Inspect its logs; the backup is retained and no reset was attempted.")


def migrate_schema(deploy: Deployment) -> None:
    deploy.sql("CREATE TABLE IF NOT EXISTS characters.skatecraft_schema_migrations "
               "(version INT UNSIGNED NOT NULL PRIMARY KEY, applied_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP);")
    version = deploy.sql("SELECT COALESCE(MAX(version),0) FROM characters.skatecraft_schema_migrations;")
    if int(version) > SCHEMA_VERSION:
        raise ValueError("The Skatecraft database schema is newer than this checkout; refusing a downgrade.")
    for table, columns in TABLES.items():
        deploy.sql(f"CREATE TABLE IF NOT EXISTS characters.{table} ({columns});")
        names = [column.strip().split()[0] for column in columns.split(",")]
        # Fail on an incompatible pre-existing table, without overwriting any of its rows.
        deploy.sql(f"SELECT {','.join(names)} FROM characters.{table} LIMIT 0;")
    deploy.sql(f"INSERT IGNORE INTO characters.skatecraft_schema_migrations (version) VALUES ({SCHEMA_VERSION});")


def core_migration_limits(image: str) -> dict[str, str]:
    limits = {}
    for schema, source in (("mangos", "world"), ("characters", "characters"), ("realmd", "logon"), ("logs", "logs")):
        result = subprocess.run(["docker", "run", "--rm", "--network", "none", "--entrypoint", "grep", image,
                                 "-Eo", r"WHERE `id`='[0-9]{14}'", f"/sql/migrations/{source}_db_updates.sql"],
                                capture_output=True, text=True)
        if result.returncode not in (0, 1):
            raise ValueError("Unable to read the pinned image's schema migration versions.")
        limits[schema] = max(re.findall(r"[0-9]{14}", result.stdout), default="")
    return limits


def refuse_newer_core(deploy: Deployment, limits: dict[str, str]) -> None:
    for schema, limit in limits.items():
        version = deploy.sql(f"SELECT COALESCE(MAX(id),'') FROM {schema}.migrations WHERE id REGEXP '^[0-9]{{14}}$';")
        if version > limit:
            raise ValueError(f"The {schema} database has newer core migrations than this fork supports. "
                             "Keep the database and update the fork's pinned server source; an automatic downgrade is unsafe.")


def check(deploy: Deployment, live: bool = False) -> None:
    config = deploy.config()
    images = build_server.image_names()
    for name in SERVICES:
        service = config["services"][name]
        expected = images["database" if name == "database" else "server"]
        if service["image"] != expected or not build_server.image_matches(expected):
            raise ValueError(f"{name} is not this checkout's patched image. Run setup again or migrate_database.py upgrade.")
        if str(service.get("environment", {}).get("VMANGOS_DEPLOY_VERSION")) != build_server.DEPLOY_VERSION:
            raise ValueError("The deployment configuration needs upgrading. Run setup again.")
    if live:
        validate_existing(config, project_containers())
        for name in SERVICES:
            ids = deploy.call("ps", "-q", name, capture_output=True, text=True).stdout.split()
            if len(ids) != 1:
                raise ValueError(f"{name} is not running. Run setup again.")
            actual = docker_json("inspect", ids[0])[0]
            expected = docker_json("image", "inspect", config["services"][name]["image"])[0]["Id"]
            if actual["Image"] != expected or actual["State"].get("Health", {}).get("Status") != "healthy":
                raise ValueError(f"{name} is not running the healthy patched image. Run setup again.")
        for table, columns in TABLES.items():
            names = [column.strip().split()[0] for column in columns.split(",")]
            deploy.sql(f"SELECT {','.join(names)} FROM characters.{table} LIMIT 0;")
        if deploy.sql(f"SELECT COUNT(*) FROM characters.skatecraft_schema_migrations WHERE version={SCHEMA_VERSION}") != "1":
            raise ValueError("The database migration has not completed. Run setup again.")


def upgrade(deploy: Deployment) -> None:
    original = deploy.config()
    container = validate_existing(original, project_containers())
    images = build_server.image_names()
    updated = normalize_config(original, images)  # Validate before building or stopping anything.
    validate_server_settings(original)
    build_server.build_images(int(os.environ.get("SKATECRAFT_BUILD_JOBS", "1")))
    limits = core_migration_limits(images["database"])
    previous_counts = None
    if container and container["State"]["Running"]:
        try:
            previous_counts = deploy.sql("SELECT COUNT(*) FROM characters.characters; SELECT COUNT(*) FROM realmd.account;")
        except subprocess.CalledProcessError:
            pass  # The complete cold backup also works for the reported version-check startup failure.
        if previous_counts is not None:
            refuse_newer_core(deploy, limits)
    backup_path = backup(deploy, original, container)
    try:
        write_config(deploy.compose, updated)
        deploy.call("up", "-d", "--no-deps", "database")
        wait_database(deploy)
        refuse_newer_core(deploy, limits)
        before = deploy.sql("SELECT COUNT(*) FROM characters.characters; SELECT COUNT(*) FROM realmd.account;")
        if previous_counts is not None and previous_counts != before:
            raise ValueError("Account/character counts changed during the upstream upgrade; inspect the retained backup.")
        migrate_schema(deploy)
        after = deploy.sql("SELECT COUNT(*) FROM characters.characters; SELECT COUNT(*) FROM realmd.account;")
        if before != after:
            raise ValueError("Account/character counts changed during the mod migration; inspect the backup before continuing.")
        save_server(deploy.server)
        print(f"Database migration complete: {after.splitlines()[0]} characters retained. "
              "Accounts and passwords are unchanged. Missing abilities are learned on login.", flush=True)
    except Exception:
        print(f"Upgrade stopped. Backup: {backup_path or 'fresh installation'}. "
              "The world server remains stopped. Do not delete the database volume.", file=sys.stderr)
        raise


def extract(deploy: Deployment, wow: Path) -> None:
    check(deploy)
    config = deploy.config()
    service = config["services"]["mangosd"]
    target = next(v for v in service["volumes"] if v["target"] == "/opt/vmangos/storage/data")
    if target["type"] != "bind":
        raise ValueError("Map extraction requires a bind-mounted server data folder.")
    Path(target["source"]).mkdir(parents=True, exist_ok=True)
    workspace = STATE / "client-extraction"
    workspace.mkdir(parents=True, exist_ok=True)
    command = ["docker", "run", "--rm"]
    for key in ("VMANGOS_DEPLOY_VERSION", "VMANGOS_UID", "VMANGOS_GID"):
        command += ["-e", f"{key}={service['environment'][key]}"]
    command += mount_args({"type": "bind", "source": str(workspace)}, "/opt/vmangos/storage/client-data")
    command += mount_args({"type": "bind", "source": str(wow.resolve() / "Data")},
                          "/opt/vmangos/storage/client-data/Data", readonly=True)
    command += mount_args(target, "/opt/vmangos/storage/data")
    run(command + [service["image"], "extract-client-data", "--force"], stdin=subprocess.DEVNULL)


def data_path(deploy: Deployment) -> Path:
    mounts = [v for v in deploy.config()["services"]["mangosd"]["volumes"]
              if v["target"] == "/opt/vmangos/storage/data"]
    if len(mounts) != 1 or mounts[0]["type"] != "bind":
        raise ValueError("Expected one bind mount for the server's extracted data.")
    return Path(mounts[0]["source"])


def maps_ready(deploy: Deployment) -> bool:
    path = data_path(deploy)
    return all((path / part).is_dir() and any(item.is_file() for item in (path / part).iterdir())
               for part in ("maps", "vmaps", "mmaps", "5875/dbc"))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("path", "upgrade", "check", "accounts", "extract", "restore", "data-path", "maps-ready"))
    parser.add_argument("--server", type=Path)
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--wow", type=Path)
    parser.add_argument("--backup", type=Path)
    args = parser.parse_args()
    server = select_server(args.server)
    deploy = Deployment(server)
    if args.command == "path":
        save_server(server)
        print(server)
    elif args.command == "upgrade":
        upgrade(deploy)
    elif args.command == "check":
        check(deploy, args.live)
        print("Verified matching patched server/database images" + (" and live migration." if args.live else "."))
    elif args.command == "accounts":
        print(deploy.sql("SELECT COUNT(*) FROM realmd.account;"))
    elif args.command == "data-path":
        print(data_path(deploy))
    elif args.command == "maps-ready":
        print(int(maps_ready(deploy)))
    elif args.command == "extract":
        if not args.wow:
            parser.error("extract requires --wow")
        extract(deploy, args.wow)
    elif args.command == "restore":
        if not args.backup:
            parser.error("restore requires --backup")
        restore(deploy, args.backup)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        # Do not print captured Compose output: it can contain database credentials.
        detail = str(error) if not isinstance(error, subprocess.CalledProcessError) else "A Docker/Git/database command failed; inspect the service logs."
        sys.exit("Skatecraft: " + detail)
