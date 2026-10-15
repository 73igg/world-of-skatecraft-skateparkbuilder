"""Regression tests for preserving existing deployments and migration failure handling."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

import build_server
import migrate_database as migration


def legacy_config() -> dict:
    return {
        "name": "world-of-skatecraft",
        "services": {
            "database": {
                "image": "ghcr.io/mserajnik/vmangos-database",
                "environment": {"MARIADB_USER": "myuser", "MARIADB_PASSWORD": "custom$pass'word",
                                "MARIADB_ROOT_PASSWORD": "different$root", "TZ": "Europe/London"},
                "volumes": [{"type": "volume", "source": "vmangos-database", "target": "/var/lib/mysql"},
                            {"type": "bind", "source": "/my/custom-sql", "target": "/sql/custom", "read_only": True}],
            },
            "realmd": {"image": "ghcr.io/mserajnik/vmangos-server:5875", "user": "1234:100",
                       "command": ["realmd"], "environment": {"TZ": "Europe/London"},
                       "ports": [{"target": 3724, "published": "13724", "protocol": "tcp"}]},
            "mangosd": {"image": "world-of-skatecraft/vmangos-server:5875-old", "user": "1234:100",
                        "command": ["mangosd"], "environment": {"TZ": "Europe/London"},
                        "volumes": [{"type": "bind", "source": "/my/maps", "target": "/opt/vmangos/storage/data"}]},
            "phpmyadmin": {"image": "phpmyadmin", "environment": {"PMA_PASSWORD": "keep$me"}},
        },
        "volumes": {"vmangos-database": {"name": "original-install_characters"}},
    }


class MigrationTests(unittest.TestCase):
    def test_legacy_upgrade_preserves_storage_credentials_and_customizations(self):
        old = legacy_config()
        original = copy.deepcopy(old)
        new = migration.normalize_config(old, build_server.image_names())
        self.assertEqual(old, original, "preflight must not mutate the original")
        self.assertEqual(migration.db_mount(new), migration.db_mount(old))
        self.assertEqual(new["volumes"], old["volumes"])
        self.assertEqual(new["services"]["phpmyadmin"], old["services"]["phpmyadmin"])
        for key in ("MARIADB_USER", "MARIADB_PASSWORD", "MARIADB_ROOT_PASSWORD", "TZ"):
            self.assertEqual(new["services"]["database"]["environment"][key], old["services"]["database"]["environment"][key])
        for name in ("realmd", "mangosd"):
            service = new["services"][name]
            self.assertNotIn("user", service)
            self.assertEqual(service["environment"]["VMANGOS_UID"], "1234")
            self.assertEqual(service["environment"]["VMANGOS_GID"], "100")
            self.assertEqual(service["environment"]["VMANGOS_DATABASE_PASSWORD"], "custom$pass'word")
            self.assertEqual(service["environment"]["VMANGOS_DEPLOY_VERSION"], "1")
        self.assertEqual(new["services"]["realmd"]["ports"], old["services"]["realmd"]["ports"])
        self.assertEqual(new["services"]["database"]["environment"]["VMANGOS_ENABLE_AUTOMATIC_WORLD_DB_CORRECTIONS"], "0")

    def test_normalization_is_repeatable_and_updates_tool_services(self):
        old = legacy_config()
        old["services"]["extract-client-data"] = {"image": "upstream", "environment": {"VMANGOS_DEPLOY_VERSION": "1"}, "profiles": ["tools"]}
        once = migration.normalize_config(old, build_server.image_names())
        twice = migration.normalize_config(once, build_server.image_names())
        self.assertEqual(once, twice)
        self.assertEqual(once["services"]["extract-client-data"]["image"], once["services"]["mangosd"]["image"])

    def test_bind_database_and_external_volume_are_kept(self):
        config = legacy_config()
        config["volumes"]["vmangos-database"]["external"] = True
        result = migration.normalize_config(config, build_server.image_names())
        self.assertTrue(result["volumes"]["vmangos-database"]["external"])
        config["services"]["database"]["volumes"][0] = {"type": "bind", "source": "/my/db", "target": "/var/lib/mysql"}
        result = migration.normalize_config(config, build_server.image_names())
        self.assertEqual(migration.db_mount(result)["source"], "/my/db")

    def test_refuses_missing_storage_newer_deployment_and_custom_entrypoint(self):
        for change in (lambda c: c["services"]["database"].update(volumes=[]),
                       lambda c: c["services"]["mangosd"]["environment"].update(VMANGOS_DEPLOY_VERSION="2"),
                       lambda c: c["services"]["mangosd"].update(entrypoint=["unpatched-server"]),
                       lambda c: c["services"]["database"].update(command=["skip-migrations"])):
            config = legacy_config()
            change(config)
            with self.assertRaises(ValueError):
                migration.normalize_config(config, build_server.image_names())

    def test_refuses_to_silently_switch_an_existing_volume(self):
        container = {"Mounts": [{"Destination": "/var/lib/mysql", "Type": "volume", "Name": "players-original-database"}]}
        with self.assertRaisesRegex(ValueError, "different storage"):
            migration.validate_existing(legacy_config(), [container])

    def test_windows_docker_bind_paths_compare_correctly(self):
        self.assertEqual(migration.canonical_mount("Z:\\Games\\Database"),
                         migration.canonical_mount("/run/desktop/mnt/host/z/Games/Database"))

    def test_password_dollars_survive_real_compose_roundtrip(self):
        # Compose itself catches escaping mistakes that would silently change existing credentials.
        if not shutil.which("docker") or subprocess.run(["docker", "compose", "version"], capture_output=True).returncode:
            self.skipTest("Docker Compose is not installed")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "compose.yaml"
            config = migration.normalize_config(legacy_config(), build_server.image_names())
            migration.write_config(path, config)
            rendered = migration.Deployment(Path(directory)).config()
            self.assertEqual(rendered["services"]["database"]["environment"]["MARIADB_ROOT_PASSWORD"], "different$root")
            self.assertEqual(rendered["services"]["mangosd"]["environment"]["VMANGOS_DATABASE_PASSWORD"], "custom$pass'word")
            migration.write_config(path, rendered)
            self.assertEqual(migration.Deployment(Path(directory)).config(), rendered)

    def test_schema_does_not_overwrite_player_records_or_preferences(self):
        deploy = Mock()
        deploy.sql.return_value = "0"
        migration.migrate_schema(deploy)
        queries = [call.args[0] for call in deploy.sql.call_args_list]
        self.assertTrue(all(not any(word in query.upper() for word in ("DELETE ", "DROP ", "REPLACE ", "TRUNCATE ", "UPDATE ")) for query in queries))
        self.assertTrue(all("character_spell" not in query for query in queries))
        for table in migration.TABLES:
            self.assertTrue(any(f"CREATE TABLE IF NOT EXISTS characters.{table}" in query for query in queries))

    def test_newer_mod_schema_is_refused(self):
        deploy = Mock()
        deploy.sql.return_value = "999"
        with self.assertRaisesRegex(ValueError, "newer"):
            migration.migrate_schema(deploy)
        self.assertEqual(deploy.sql.call_count, 2)

    def test_failed_backup_never_rewrites_compose_or_starts_upgraded_database(self):
        deploy = Mock()
        deploy.config.return_value = legacy_config()
        with patch.object(migration, "project_containers", return_value=[]), \
             patch.object(migration, "validate_server_settings"), \
             patch.object(build_server, "build_images"), \
             patch.object(migration, "core_migration_limits", return_value={}), \
             patch.object(migration, "backup", side_effect=RuntimeError("disk full")), \
             patch.object(migration, "write_config") as write:
            with self.assertRaisesRegex(RuntimeError, "disk full"):
                migration.upgrade(deploy)
            write.assert_not_called()
            deploy.call.assert_not_called()

    def test_newer_base_database_is_not_downgraded(self):
        deploy = Mock()
        deploy.sql.return_value = "20990101000000"
        with self.assertRaisesRegex(ValueError, "newer core migrations"):
            migration.refuse_newer_core(deploy, {"characters": "20260901000000"})

    def test_wrong_server_image_is_detected_before_launch(self):
        deploy = Mock()
        deploy.config.return_value = legacy_config()
        with self.assertRaisesRegex(ValueError, "patched image"):
            migration.check(deploy)

    def test_existing_maps_use_the_configured_mount(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            config = legacy_config()
            config["services"]["mangosd"]["volumes"][0]["source"] = str(path)
            deploy = Mock()
            deploy.config.return_value = config
            self.assertFalse(migration.maps_ready(deploy))
            for part in ("maps", "vmaps", "mmaps", "5875/dbc"):
                (path / part).mkdir(parents=True)
                (path / part / "existing-file").write_text("fixture")
            self.assertTrue(migration.maps_ready(deploy))


if __name__ == "__main__":
    unittest.main()
