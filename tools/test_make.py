"""Hermetic command-boundary tests for the repository Makefile."""

from __future__ import annotations

import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

TOOLS_DIR = Path(__file__).resolve().parent
REPO_ROOT = TOOLS_DIR.parent


class MakefileTests(unittest.TestCase):
    def run_make(
        self,
        *arguments: str,
        variables: dict[str, str] | None = None,
        environment: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        command = ["make", "--no-print-directory"]
        if variables:
            command.extend(f"{name}={value}" for name, value in variables.items())
        command.extend(arguments)
        env = os.environ.copy()
        if environment:
            env.update(environment)
        return subprocess.run(
            command,
            cwd=REPO_ROOT,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

    @staticmethod
    def fake_command(directory: Path, name: str, body: str) -> Path:
        command = directory / name
        command.write_text(f"#!/bin/sh\n{body}", encoding="utf-8")
        command.chmod(command.stat().st_mode | stat.S_IXUSR)
        return command

    @staticmethod
    def command_logger(directory: Path, name: str = "command") -> Path:
        return MakefileTests.fake_command(
            directory,
            name,
            'printf \'%s\\n\' "$*" >> "$COMMAND_LOG"\nexit 0\n',
        )

    @staticmethod
    def read_log(path: Path) -> list[str]:
        return path.read_text(encoding="utf-8").splitlines() if path.exists() else []

    def test_help_is_the_default_target(self) -> None:
        result = self.run_make()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Available targets:", result.stdout)
        self.assertIn("module-sync", result.stdout)
        self.assertIn("module-update", result.stdout)
        self.assertIn("check-generated", result.stdout)
        self.assertIn("release", result.stdout)
        self.assertIn("test", result.stdout)

    def test_dry_runs_show_each_external_invocation(self) -> None:
        expected = {
            "module-sync": "git submodule update --init --recursive -- modules/Antelope-Ctl",
            "generate": (
                "python3 tools/generate_device_catalog.py "
                "--profiles-dir modules/Antelope-Ctl/profiles "
                "--output src/device/generated.rs "
                "--pack-output src/device/generated_profiles.json"
            ),
            "check-generated": (
                "python3 tools/generate_device_catalog.py "
                "--check modules/Antelope-Ctl/profiles "
                "--generated src/device/generated.rs "
                "--pack-generated src/device/generated_profiles.json"
            ),
            "release": "cargo build --release --locked",
            "test": "cargo test --workspace",
        }

        for target, invocation in expected.items():
            with self.subTest(target=target):
                result = self.run_make("-n", target)
                self.assertEqual(result.returncode, 0, result.stderr)
                normalized_output = " ".join(result.stdout.replace(chr(92) + "\n", " ").split())
                self.assertIn(invocation, normalized_output)

        release = self.run_make("-n", "release")
        self.assertNotIn("submodule", release.stdout)
        self.assertNotIn("generate_device_catalog.py", release.stdout)

    def test_targets_invoke_only_their_mocked_command_boundary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            log = directory / "commands.log"
            fake_git = self.command_logger(directory, "git")
            fake_python = self.command_logger(directory, "python")
            fake_cargo = self.command_logger(directory, "cargo")
            environment = {"COMMAND_LOG": str(log)}

            result = self.run_make(
                "module-sync", variables={"GIT": str(fake_git)}, environment=environment
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                self.read_log(log),
                ["submodule update --init --recursive -- modules/Antelope-Ctl"],
            )

            log.unlink()
            result = self.run_make(
                "generate", variables={"PYTHON": str(fake_python)}, environment=environment
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                self.read_log(log),
                [
                    "tools/generate_device_catalog.py --profiles-dir modules/Antelope-Ctl/profiles "
                    "--output src/device/generated.rs --pack-output src/device/generated_profiles.json"
                ],
            )

            log.unlink()
            result = self.run_make(
                "check-generated",
                variables={"PYTHON": str(fake_python)},
                environment=environment,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                self.read_log(log),
                [
                    "tools/generate_device_catalog.py --check modules/Antelope-Ctl/profiles "
                    "--generated src/device/generated.rs "
                    "--pack-generated src/device/generated_profiles.json"
                ],
            )

            log.unlink()
            result = self.run_make(
                "release", variables={"CARGO": str(fake_cargo)}, environment=environment
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(self.read_log(log), ["build --release --locked"])

            log.unlink()
            result = self.run_make(
                "test", variables={"CARGO": str(fake_cargo)}, environment=environment
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(self.read_log(log), ["test --workspace"])

    def module_update_git(self, directory: Path) -> Path:
        return self.fake_command(
            directory,
            "git",
            """\
printf '%s\\n' "$*" >> "$COMMAND_LOG"
case "${FAKE_GIT_MODE:-clean}:$*" in
    status-error:*status*) exit 128 ;;
    branch-error:*symbolic-ref*) printf '%s\\n' 'profile/test'; exit 128 ;;
    upstream-error:*rev-parse*) printf '%s\\n' 'origin/profile/test'; exit 128 ;;
    dirty:*status*) printf '%s\\n' ' M profiles/example.json' ;;
    detached:*symbolic-ref*) exit 1 ;;
    no-upstream:*rev-parse*) exit 1 ;;
esac
case "$*" in
    *symbolic-ref*) printf '%s\\n' 'profile/test' ;;
    *rev-parse*) printf '%s\\n' 'origin/profile/test' ;;
esac
exit 0
""",
        )

    def assert_module_update_refuses(self, mode: str, message: str, calls: int) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            log = directory / "commands.log"
            fake_git = self.module_update_git(directory)
            result = self.run_make(
                "module-update",
                variables={"GIT": str(fake_git)},
                environment={"COMMAND_LOG": str(log), "FAKE_GIT_MODE": mode},
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn(message, result.stderr)
            self.assertEqual(len(self.read_log(log)), calls)
            self.assertNotIn("pull --ff-only", "\n".join(self.read_log(log)))

    def test_module_update_refuses_git_command_errors(self) -> None:
        for mode, message, calls in [
            ("status-error", "cannot inspect submodule status", 1),
            ("branch-error", "refusing to update a detached submodule", 2),
            ("upstream-error", "branch has no configured upstream", 3),
        ]:
            with self.subTest(mode=mode):
                self.assert_module_update_refuses(mode, message, calls)

    def test_module_update_refuses_dirty_submodule(self) -> None:
        self.assert_module_update_refuses(
            "dirty", "refusing to update a dirty submodule", calls=1
        )

    def test_module_update_refuses_detached_submodule(self) -> None:
        self.assert_module_update_refuses(
            "detached", "refusing to update a detached submodule", calls=2
        )

    def test_module_update_refuses_missing_configured_upstream(self) -> None:
        self.assert_module_update_refuses(
            "no-upstream", "branch has no configured upstream", calls=3
        )

    def test_module_update_fast_forwards_clean_branch_from_upstream(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            log = directory / "commands.log"
            fake_git = self.module_update_git(directory)
            result = self.run_make(
                "module-update",
                variables={"GIT": str(fake_git)},
                environment={"COMMAND_LOG": str(log)},
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                self.read_log(log),
                [
                    "-C modules/Antelope-Ctl status --porcelain --untracked-files=all",
                    "-C modules/Antelope-Ctl symbolic-ref --quiet --short HEAD",
                    "-C modules/Antelope-Ctl rev-parse --abbrev-ref --symbolic-full-name @{upstream}",
                    "-C modules/Antelope-Ctl pull --ff-only",
                ],
            )


if __name__ == "__main__":
    unittest.main()
