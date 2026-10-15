"""Opt-in Docker test: SKATECRAFT_TEST_DATABASE=1 python -m unittest discover -s setup -v.

Uses only a disposable, uniquely named project and synthetic player records. Requires the
matching images built by setup/build_server.py. Never connects to the player's project.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import uuid

import build_server
import migrate_database as migration


@unittest.skipUnless(os.environ.get("SKATECRAFT_TEST_DATABASE") == "1", "opt-in disposable Docker integration test")
class DatabaseIntegrationTest(unittest.TestCase):
    def test_existing_player_upgrade_repeat_and_restore(self):
        project = "skatecraft-migration-test-" + uuid.uuid4().hex[:10]
        images = build_server.image_names()
        self.assertTrue(all(build_server.image_matches(image) for image in images.values()), "build the matching images first")
        state_root = migration.ROOT / ".setup"
        state_root.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="migration-test-", dir=state_root) as directory:
            server = Path(directory).resolve()
            self.assertTrue(server.is_relative_to(state_root.resolve()))
            (server / "config").mkdir()
            (server / "private-state").mkdir()
            for name in ("realmd", "mangosd"):
                shutil.copy2(build_server.source_checkout() / f"config/{name}.conf.example", server / f"config/{name}.conf")
            services = {
                "database": {
                    "cpus": 1, "mem_limit": "1g",
                    "image": images["database"], "environment": {"VMANGOS_DEPLOY_VERSION": "1",
                        "MARIADB_ROOT_PASSWORD": "test$root'password", "MARIADB_USER": "mangos", "MARIADB_PASSWORD": "test$db'password"},
                    "healthcheck": {"test": ["CMD", "vmangos-healthcheck"], "interval": "2s", "timeout": "3s", "retries": 100},
                    "volumes": [{"type": "volume", "source": "database", "target": "/var/lib/mysql"}],
                },
            }
            for name in ("mangosd", "realmd"):
                services[name] = {"image": images["server"], "command": [name], "user": "1000:1000",
                                  "volumes": [{"type": "bind", "source": str(server / f"config/{name}.conf"),
                                               "target": f"/opt/vmangos/config/{name}.conf", "read_only": True}]}
            config = {"services": services, "volumes": {"database": {"name": project + "-database"}}}
            migration.write_config(server / "compose.yaml", config)
            with patch.object(migration, "PROJECT", project), patch.object(migration, "STATE", server / "private-state"):
                deploy = migration.Deployment(server)
                created_volumes = {project + "-database"}
                try:
                    deploy.call("up", "-d", "database")
                    migration.wait_database(deploy)
                    deploy.sql("INSERT INTO realmd.account (id,username,s,v) VALUES (101,'KEEPME','abc123','def456');"
                               "INSERT INTO characters.characters (guid,account,name,level,money,map,position_x) "
                               "VALUES (101,101,'Keptskater',42,987654,0,-1234.5);"
                               "INSERT INTO characters.character_spell (guid,spell,active,disabled) VALUES (101,60001,1,0);"
                               "INSERT INTO characters.item_instance (guid,item_id,owner_guid,enchantments,durability) VALUES (201,25,101,'',21);"
                               "INSERT INTO characters.character_inventory (guid,slot,item_guid,item_id) VALUES (101,15,201,25);"
                               "INSERT INTO characters.character_queststatus (guid,quest,status,mob_count1) VALUES (101,1,1,3);"
                               "INSERT INTO characters.character_action (guid,button,action,type) VALUES (101,0,60001,0);"
                               "CREATE TABLE mangos.my_custom_world_edit (id INT PRIMARY KEY, note TEXT);"
                               "INSERT INTO mangos.my_custom_world_edit VALUES (1,'keep custom world data');"
                               "CREATE TABLE characters.character_skatecraft_speed (" + migration.TABLES["character_skatecraft_speed"] + ");"
                               "INSERT INTO characters.character_skatecraft_speed VALUES (101,27);"
                               "CREATE TABLE characters.character_skatecraft_flag (" + migration.TABLES["character_skatecraft_flag"] + ");"
                               "INSERT INTO characters.character_skatecraft_flag VALUES (101,3,0,0,1,1.5,2.5,3.5,0.5);")
                    snapshot_query = ("SELECT * FROM realmd.account WHERE id=101;"
                                      "SELECT * FROM characters.characters WHERE guid=101;"
                                      "SELECT * FROM characters.character_spell WHERE guid=101;"
                                      "SELECT * FROM characters.item_instance WHERE owner_guid=101;"
                                      "SELECT * FROM characters.character_inventory WHERE guid=101;"
                                      "SELECT * FROM characters.character_queststatus WHERE guid=101;"
                                      "SELECT * FROM characters.character_action WHERE guid=101;"
                                      "SELECT * FROM characters.character_skatecraft_speed WHERE guid=101;"
                                      "SELECT * FROM characters.character_skatecraft_flag WHERE guid=101;"
                                      "SELECT * FROM mangos.my_custom_world_edit;")
                    before = deploy.sql(snapshot_query)
                    # Simulate the base deployment's old configuration and lack of mod tables.
                    old = deploy.config()
                    old["services"]["database"]["environment"].pop("VMANGOS_DEPLOY_VERSION")
                    migration.write_config(deploy.compose, old)
                    migration.upgrade(deploy)
                    self.assertEqual(deploy.sql(snapshot_query), before)
                    self.assertEqual(deploy.sql("SELECT COUNT(*) FROM characters.skatecraft_schema_migrations"), "1")
                    first_backup = next((server / "private-state/database-backups").iterdir())
                    self.assertTrue(json.loads((first_backup / "manifest.json").read_text())["complete"])
                    self.assertGreater((first_backup / "database-volume.tar.gz").stat().st_size, 0)
                    self.assertTrue((first_backup / "databases.sql").is_file())
                    migration.upgrade(deploy)
                    self.assertEqual(deploy.sql(snapshot_query), before)
                    self.assertEqual(deploy.sql("SELECT COUNT(*) FROM characters.skatecraft_schema_migrations"), "1")
                    # Restoring uses a new volume and also recovers the old credentials containing dollars/quotes.
                    deploy.sql("UPDATE characters.characters SET money=1 WHERE guid=101;")
                    migration.restore(deploy, first_backup)
                    restored = migration.db_mount(deploy.config())["source"]
                    self.assertNotEqual(restored, project + "-database")
                    created_volumes.add(restored)
                    # The first backup has the old configuration, which predates the version variable.
                    restored_config = deploy.config()
                    restored_config["services"]["database"]["environment"]["VMANGOS_DEPLOY_VERSION"] = "1"
                    migration.write_config(deploy.compose, restored_config)
                    deploy.call("up", "-d", "database")
                    migration.wait_database(deploy)
                    self.assertEqual(deploy.sql(snapshot_query), before)
                finally:
                    self.assertTrue(project.startswith("skatecraft-migration-test-"))
                    deploy.call("down", "--remove-orphans")
                    # Only names generated by this test may be removed; the player's volumes are never enumerated.
                    for volume in created_volumes:
                        self.assertTrue(volume.startswith(project))
                        subprocess.run(["docker", "volume", "rm", volume], check=True, capture_output=True)


if __name__ == "__main__":
    unittest.main()
