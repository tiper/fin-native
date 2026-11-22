#!/usr/bin/env python3
"""The PLATFORM build axis (the Makefile's `PLATFORM`, `ci/platform_axis.py`): what it accepts, what it
implies, what it must not change, and that its script is not named after the standard library.

* `ci/platform_axis.py --list` names `webos desktop tizen`, and the Makefile reads that list the way it
  reads FLAVOR's from `ci/flavor.py --list`.
* `PLATFORM` defaults to `webos`, takes any of the three from the command line or the environment, and
  refuses anything else at parse time: an unknown name, an empty one, two words, a `%` (a `filter` of the
  value against the list would pass the last two).
* The refusal comes before the Makefile's stamp reset, whatever the goal: a mistyped `PLATFORM` beside a
  goal that builds must not delete the last build's binary. That is read from a seeded scratch tree, by
  the files surviving (a refusal below the reset would still print the same error).
* `sim`, `sim-*`, `macapp` and `macapp-zip` imply `desktop` when `PLATFORM` was not given and an explicit
  value wins. That depends on the GOALS, and `make -s print-platform sim-macos` would BUILD the simulator,
  so it is read from a dry run (`make -n`, which prints the recipe line `echo 'desktop'` instead of running
  it) in a scratch tree: a dry run beside a goal that builds is not a pure query, so its parse-time reset
  of `pkg/` (see ci/test_arm_profile.py) must not reach this checkout.
* The goals that build the webOS television package (`WEBOS_TV_GOALS` in the Makefile: `all`, `ipk`, `deploy`, `install`, the
  storage helper, the C units, `libass`...) are REFUSED under an explicit `PLATFORM=desktop` or `PLATFORM=tizen`, at parse time
  above the stamp reset (so a refusal deletes nothing), with a message naming the goal and the platform. The same goals under
  the default `webos` print the recipe text they printed before the refusal existed (the Makefile with the guard cut out is
  the control). The simulators keep their implied `desktop`; `make sim all` mixes a desktop goal with a webOS one and is
  refused (one PLATFORM per invocation), `PLATFORM=webos make sim all` is not; the queries answer under every value, and a
  bare `make PLATFORM=tizen` still defaults to `tizen-check`.
* The compatibility axis is not the platform axis: `print-rust-target` answers the same under every
  `PLATFORM`.
* Both new query goals are in the Makefile's side-effect-free set: asking resets nothing under `pkg/`. A
  control proves the seeded tree below would have been reset by a goal that builds.
* The script's name. Scripts in ci/ `import platform` and Python puts a script's own directory first on
  `sys.path`, so a `ci/platform.py` would replace the standard library module for all of them. A probe run
  from `ci/` must get the real one, each existing consumer must import it from there, and a control proves
  the probe would notice a shadow.

Pure queries run from the checkout. Everything that dry-runs a goal runs in a scratch tree that symlinks the
checkout's files but owns real `pkg/` and `vendor/` directories, and never dry-runs a goal whose recipe
re-enters make (`check`, `install`...): `$(MAKE)` lines run even under `-n`.
"""
from __future__ import annotations

import ast
import importlib.util
import os
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CI = ROOT / "ci"
PLATFORMS = ("webos", "desktop", "tizen")
RUST_TARGET = "armv7-unknown-linux-gnueabi"
# What the Makefile reads the environment for, which a developer's shell or an outer make may have set.
SCRUBBED = ("MAKEFLAGS", "MFLAGS", "MAKELEVEL", "PLATFORM", "FLAVOR", "RELEASE", "SYMBOLS", "LAB", "CI",
            "GITHUB_ACTIONS", "ARM_PROFILE", "TV", "PLAYBACK_BACKENDS", "SIM_TDIR", "MACAPP_TDIR")
# The goals the Makefile implies `desktop` for, one per kind of recipe: the simulators, their aliases and
# their helpers, and the macOS app. Every one is dry-run below; none of them re-enters make.
DESKTOP_GOALS = ("sim", "sim-macos", "sim-linux", "sim-wsl", "sim-run", "sim-macos-run", "sim-shot",
                 "sim-macos-shot", "sim-token", "sim-macos-token", "sim-play", "sim-macos-play", "sim-clean",
                 "sim-macos-clean", "macapp", "macapp-zip")
# Goals that build the television package: they stay on the default. (Not `check` or `install`, whose
# recipes run `$(MAKE)` even under `-n`.)
WEBOS_GOALS = ("all", "ipk")
# What the Makefile refuses under another platform: one per kind of recipe (the package, the helper, a C unit, the deploy and
# install, the symbol and library builds). The Makefile's own `WEBOS_TV_GOALS` list is read and compared below.
REFUSED_GOALS = ("all", "ipk", "deploy", "install", "test", "symbols", "symbols-libs", "sentry-symbols", "libass",
                 "sentry-native", "pkg/plxnative", "pkg/plxnative-storage", "src/main.o", "src/starfish.o")
REFUSAL = "builds the webOS television package"
STAMP = "pkg/.build-config"
BINARY = "pkg/plxnative"
SEEDED_STAMP = "seeded by test_platform_axis"


def environment(extra: dict[str, str] | None = None) -> dict[str, str]:
    env = {k: v for k, v in os.environ.items() if k not in SCRUBBED}
    env.update(extra or {})
    return env


def make(*args: str, cwd: Path = ROOT, dry: bool = False, env: dict[str, str] | None = None
         ) -> subprocess.CompletedProcess:
    cmd = ["make", "-n" if dry else "", "-s", "--no-print-directory", "-C", str(cwd), *args]
    return subprocess.run([c for c in cmd if c], capture_output=True, text=True, env=environment(env),
                          timeout=120)


class PlatformList(unittest.TestCase):
    def run_script(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run([sys.executable, str(CI / "platform_axis.py"), *args], capture_output=True,
                              text=True, timeout=30)

    def test_the_list_names_the_three_platforms(self):
        proc = self.run_script("--list")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(proc.stdout, "webos desktop tizen\n")

    def test_anything_else_is_a_usage_error(self):
        for args in ((), ("--bogus",), ("--list", "extra"), ("webos",)):
            with self.subTest(args=args):
                proc = self.run_script(*args)
                self.assertEqual(proc.returncode, 2)
                self.assertNotEqual(proc.stdout.splitlines()[:1], ["webos desktop tizen"])

    def test_it_imports_nothing_but_sys(self):
        """Every `make` invocation spawns it (it is the FLAVOR cost again), and each import adds to the bill."""
        tree = ast.parse((CI / "platform_axis.py").read_text())
        imported = {alias.name for node in ast.walk(tree) if isinstance(node, ast.Import) for alias in node.names}
        imported |= {node.module for node in ast.walk(tree) if isinstance(node, ast.ImportFrom)}
        self.assertEqual(imported, {"sys"})

    def test_the_makefile_reads_the_list_the_way_it_reads_flavors(self):
        text = (ROOT / "Makefile").read_text()
        self.assertRegex(text, r"(?m)^PLATFORMS\s*:=\s*\$\(shell python3 ci/platform_axis\.py --list\)\s*$")
        self.assertRegex(text, r"(?m)^PLATFORM\s*\?=\s*webos\s*$")


class PlatformVariable(unittest.TestCase):
    def query(self, *args: str, env: dict[str, str] | None = None) -> str:
        proc = make("print-platform", *args, env=env)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(proc.stderr, "")
        return proc.stdout.strip()

    def test_the_default_is_webos(self):
        self.assertEqual(self.query(), "webos")

    def test_every_listed_platform_is_accepted_from_the_command_line_and_the_environment(self):
        for platform in PLATFORMS:
            with self.subTest(platform=platform):
                self.assertEqual(self.query(f"PLATFORM={platform}"), platform)
                self.assertEqual(self.query(env={"PLATFORM": platform}), platform)

    def test_anything_else_is_refused_at_parse_time(self):
        for value in ("webso", "WEBOS", "", "webos bogus", "webos webos", "%", "web%", "android"):
            with self.subTest(value=value):
                proc = make("print-platform", f"PLATFORM={value}")
                self.assertNotEqual(proc.returncode, 0)
                self.assertEqual(proc.stdout, "")
                self.assertIn(f'unknown PLATFORM "{value}"', proc.stderr)
                self.assertIn("one of: webos desktop tizen", proc.stderr)

    def test_the_rust_target_is_not_chosen_by_the_platform(self):
        """The compatibility axis (Rust target, sysroot, TFM, firmware floor, signing) is separate."""
        for platform in PLATFORMS:
            with self.subTest(platform=platform):
                proc = make("print-rust-target", f"PLATFORM={platform}")
                self.assertEqual(proc.returncode, 0, proc.stderr)
                self.assertEqual(proc.stdout.strip(), RUST_TARGET)
        self.assertEqual(make("print-rust-target").stdout.strip(), RUST_TARGET)

    def test_the_existing_queries_do_not_move(self):
        """`make -s print-flavor print-appid` is what every tool asks, and PLATFORM changes neither."""
        for platform in PLATFORMS:
            with self.subTest(platform=platform):
                proc = make("print-flavor", "print-appid", "print-appport", f"PLATFORM={platform}")
                self.assertEqual(proc.returncode, 0, proc.stderr)
                self.assertEqual(proc.stdout.split(), ["debug", "com.beb.plxnative.debug", "8911"])


class ScratchTree(unittest.TestCase):
    """A scratch checkout to dry-run goals in. It holds symlinks to the checkout's own files (the Makefile
    reads `ci/` and `tools/` while it parses) and REAL `pkg/` and `vendor/` directories, because the
    parse-time reset removes paths under exactly those two and, through a symlink, would delete this
    checkout's binary and its FFmpeg header sentinel (ci/test_arm_profile.py has the whole account)."""

    def setUp(self):
        td = tempfile.TemporaryDirectory(prefix="plx-platform.")
        self.addCleanup(td.cleanup)
        self.tree = Path(td.name)
        for entry in ROOT.iterdir():
            if entry.name not in (".git", "pkg", "vendor"):
                (self.tree / entry.name).symlink_to(entry)
        (self.tree / "pkg").mkdir()
        (self.tree / "vendor").mkdir()
        # The package's tracked inputs, so `ipk` has what it names; build outputs stay out.
        (self.tree / "pkg/appinfo.json").write_text((ROOT / "pkg/appinfo.json").read_text())

    def dry(self, *args: str, env: dict[str, str] | None = None) -> list[str]:
        """The recipe lines a dry run printed. `-n` prints even an `@`-silenced line, minus the `@`."""
        proc = make(*args, cwd=self.tree, dry=True, env=env)
        self.assertEqual(proc.returncode, 0, (args, proc.stderr[-400:]))
        return proc.stdout.splitlines()

    def seed(self) -> None:
        """The tree the way a finished build leaves it: a stamp that is not the current configuration's (so
        the parse-time reset would rewrite it) and a built binary (which the reset would delete)."""
        (self.tree / STAMP).write_text(SEEDED_STAMP)
        (self.tree / BINARY).write_text("a built binary\n")

    def assert_untouched(self, what: str) -> None:
        self.assertEqual((self.tree / STAMP).read_text(), SEEDED_STAMP, f"{what} rewrote the build stamp")
        self.assertTrue((self.tree / BINARY).exists(), f"{what} deleted a built binary")


class ImpliedPlatform(ScratchTree):
    def platform_of(self, *goals: str, env: dict[str, str] | None = None) -> str:
        """The value `print-platform` would print beside these goals: its recipe line `echo '<value>'`."""
        shown = [m.group(1) for line in self.dry("print-platform", *goals, env=env)
                 if (m := re.fullmatch(r"echo '(webos|desktop|tizen)'", line))]
        self.assertEqual(len(shown), 1, (goals, shown))
        return shown[0]

    def test_the_simulators_and_the_macos_app_imply_desktop(self):
        for goal in DESKTOP_GOALS:
            with self.subTest(goal=goal):
                self.assertEqual(self.platform_of(goal), "desktop")

    def test_goals_that_build_the_television_package_stay_on_the_default(self):
        for goal in WEBOS_GOALS:
            with self.subTest(goal=goal):
                self.assertEqual(self.platform_of(goal), "webos")
        self.assertEqual(self.platform_of(), "webos")

    def test_a_goal_that_is_not_one_of_them_does_not_imply_it(self):
        self.assertEqual(self.platform_of("print-appid"), "webos")
        self.assertEqual(self.platform_of("print-appid", "ipk"), "webos")

    def test_an_explicit_value_wins_over_the_implication(self):
        self.assertEqual(self.platform_of("sim-macos", "PLATFORM=webos"), "webos")
        self.assertEqual(self.platform_of("macapp", "PLATFORM=tizen"), "tizen")
        self.assertEqual(self.platform_of("sim-macos", env={"PLATFORM": "webos"}), "webos")
        self.assertEqual(self.platform_of("print-appid", "PLATFORM=desktop"), "desktop")

    def test_asking_never_resets_a_built_tree_and_a_building_goal_does(self):
        """Seed a tree the way a finished build leaves it, then ask the two new questions: nothing moves. The
        control (a goal that builds, beside the same query) must reset the same files, or this proves nothing."""
        for goal in ("print-platform", "print-rust-target"):
            with self.subTest(goal=goal):
                self.seed()
                for args in ((goal,), (goal, "PLATFORM=desktop"), (goal, "PLATFORM=tizen")):
                    proc = make(*args, cwd=self.tree)
                    self.assertEqual(proc.returncode, 0, proc.stderr)
                self.assert_untouched(f"make {goal}")
        self.seed()
        self.dry("print-platform", "all")
        self.assertFalse((self.tree / BINARY).exists(), "control: a building goal must reset the seeded tree")
        self.assertNotEqual((self.tree / STAMP).read_text(), SEEDED_STAMP)

    def test_the_queries_are_registered_as_queries(self):
        text = (ROOT / "Makefile").read_text().replace("\\\n", " ")
        for goal in ("print-platform", "print-rust-target"):
            with self.subTest(goal=goal):
                self.assertRegex(text, rf"(?m)^QUERY_GOALS\s*=[^\n]*\b{goal}\b")
                self.assertRegex(text, rf"(?m)^{goal}:")


class TizenConfiguration(unittest.TestCase):
    """`mk/platform-tizen.mk`'s own variables, read with `print-tizen-config` (a query: it touches nothing)."""

    def config(self, *args: str) -> dict[str, str]:
        proc = make("PLATFORM=tizen", *args, "print-tizen-config")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        return dict(line.split("=", 1) for line in proc.stdout.splitlines() if "=" in line)

    def test_a_release_library_carries_no_dev_trigger_seam(self):
        """The webOS rule (`RELEASE=1` builds without `devtriggers`): a library that ships must not arm the dev boot."""
        self.assertEqual(self.config()["TIZEN_FEATURES"], "tizen,uri,devtriggers")
        self.assertEqual(self.config("RELEASE=1")["TIZEN_FEATURES"], "tizen,uri")
        self.assertEqual(self.config("RELEASE=1")["TIZEN_PROFILE"], "release")
        self.assertEqual(self.config("RELEASE=")["TIZEN_FEATURES"], "tizen,uri,devtriggers")

    def test_the_shipped_tizen_build_plays_and_a_no_driver_build_stays_possible(self):
        """The Tizen library is built WITH its own `uri` feature (the URI driver over the host's media player), never with the
        application's `play-uri` or `play-feed`; `TIZEN_URI=0` is the way to build without it."""
        for args in ((), ("RELEASE=1",)):
            features = self.config(*args)["TIZEN_FEATURES"].split(",")
            self.assertIn("uri", features)
            self.assertFalse({"play-uri", "play-feed"} & set(features), features)
        self.assertEqual(self.config("TIZEN_URI=0")["TIZEN_FEATURES"], "tizen,devtriggers")
        self.assertEqual(self.config("TIZEN_URI=0", "RELEASE=1")["TIZEN_FEATURES"], "tizen")


    def test_the_verification_comes_after_the_link(self):
        """Under -j a verify that is a sibling of the link reads the OLD library. In a dry run the recipes print in the order they would
        run, and `-j` with a serial dry run still shows the dependency order make enforces."""
        for goals in (("tizen-tpk",), ("tizen-so", "tizen-verify"), ("tizen-verify", "tizen-so")):
            with self.subTest(goals=goals):
                proc = make("PLATFORM=tizen", "-j8", *goals, dry=True)
                self.assertEqual(proc.returncode, 0, proc.stderr)
                lines = proc.stdout.splitlines()
                link = max(i for i, l in enumerate(lines) if l.startswith("mv -f tizen/out/x86_64/libplx.so.part"))
                verify = min(i for i, l in enumerate(lines) if "ci/check-tizen-so.py verify" in l)
                self.assertGreater(verify, link, "tizen-verify would run before the link finished")


class WebosGoalsRefused(ScratchTree):
    def refused(self, *args: str, env: dict[str, str] | None = None) -> str:
        """Dry-run the goals on a SEEDED tree and return the refusal text; the tree must be untouched."""
        self.seed()
        proc = make(*args, cwd=self.tree, dry=True, env=env)
        self.assertNotEqual(proc.returncode, 0, (args, proc.stdout[-200:]))
        self.assertEqual(proc.stdout, "", args)
        self.assert_untouched(f"make {' '.join(args)}")
        return proc.stderr

    def test_the_television_goals_are_refused_under_desktop_and_tizen(self):
        for platform in ("desktop", "tizen"):
            for goal in REFUSED_GOALS:
                with self.subTest(platform=platform, goal=goal):
                    err = self.refused(f"PLATFORM={platform}", goal)
                    self.assertIn(f"`{goal}` {REFUSAL}", err)
                    self.assertIn(f"refused under PLATFORM={platform}", err)
                    self.assertIn("Use PLATFORM=webos", err)

    def test_the_environment_value_is_refused_too_and_a_bare_make_asks_for_all(self):
        self.assertIn(f"`all` {REFUSAL}", self.refused(env={"PLATFORM": "desktop"}))
        self.assertIn(f"`ipk` {REFUSAL}", self.refused("ipk", env={"PLATFORM": "tizen"}))
        self.assertIn(f"`all` {REFUSAL}", self.refused("PLATFORM=desktop"))

    def test_the_message_names_every_television_goal_asked_for(self):
        err = self.refused("PLATFORM=desktop", "print-appid", "ipk", "deploy")
        self.assertIn("`ipk deploy`", err)

    def test_the_makefiles_list_is_the_one_tested(self):
        text = (ROOT / "Makefile").read_text().replace("\\\n", " ")
        listed = re.search(r"(?m)^WEBOS_TV_GOALS\s*:=\s*([^\n]+)$", text)
        self.assertIsNotNone(listed)
        names = set(listed.group(1).split())
        self.assertEqual(names, {g for g in REFUSED_GOALS if not g.endswith(".o")} | {"src/%.o"})

    def test_the_simulators_the_queries_and_the_tizen_goals_are_not_refused(self):
        for platform in ("desktop", "tizen"):
            with self.subTest(platform=platform):
                self.dry(f"PLATFORM={platform}", "print-appid", "print-platform")
                proc = make(f"PLATFORM={platform}", "print-playback-config")
                self.assertEqual(proc.returncode, 0, proc.stderr)
                self.assertTrue(proc.stdout.startswith("backends="), proc.stdout[:80])
        for goal in DESKTOP_GOALS:
            with self.subTest(goal=goal):
                self.dry(goal)
        self.dry("PLATFORM=tizen", "tizen-abi")
        self.dry("PLATFORM=desktop", "sim-macos")

    def test_a_bare_make_on_tizen_still_defaults_to_the_tizen_check(self):
        lines = self.dry("PLATFORM=tizen")
        self.assertTrue(any("-p plx_platform_tizen" in l for l in lines), lines[-3:])
        self.assertFalse(any("pkg/plxnative" in l or "src/main" in l for l in lines))

    def test_a_simulator_beside_a_television_goal_is_refused_and_the_explicit_webos_is_not(self):
        """One PLATFORM per invocation: `sim` implies desktop, so `all` beside it is refused and the message says why.
        An explicit value wins over the implication (as it always has), so `PLATFORM=webos` takes both."""
        for goals in (("sim", "all"), ("sim-macos", "ipk"), ("macapp", "deploy")):
            with self.subTest(goals=goals):
                err = self.refused(*goals)
                self.assertIn(REFUSAL, err)
                self.assertIn("refused under PLATFORM=desktop (implied by the simulator goal beside it", err)
        self.dry("PLATFORM=webos", "sim", "all")

    def test_the_webos_recipe_text_does_not_move(self):
        """The refusal is parse-time text: cut it out of the Makefile and the webOS goals print the same recipes."""
        text = (ROOT / "Makefile").read_text()
        guard = re.search(r"(?ms)^ifneq \(\$\(PLATFORM\),webos\)\n.*?^endif\nendif\n", text)
        self.assertIsNotNone(guard, "the refusal block is gone or reshaped")
        self.assertIn("$(error", guard.group(0))
        for goals in (("all",), ("ipk",), ("symbols",), ("pkg/plxnative-storage",), ("src/main.o",)):
            with self.subTest(goals=goals):
                with_guard = self.dry(*goals)
                (self.tree / "Makefile").unlink()
                (self.tree / "Makefile").write_text(text.replace(guard.group(0), ""))
                try:
                    without = self.dry(*goals)
                finally:
                    (self.tree / "Makefile").unlink()
                    (self.tree / "Makefile").symlink_to(ROOT / "Makefile")
                self.assertGreaterEqual(len(with_guard), 1, goals)
                self.assertEqual(with_guard, without)


class RefusedPlatform(ScratchTree):
    def test_a_refused_value_is_refused_whatever_the_goal(self):
        """The refusal is at parse time, above the stamp reset that deletes `pkg/plxnative` and the staged
        libraries, so a mistyped value beside a goal that builds (`all`, `sim-macos`) leaves a built tree
        alone. The error text alone cannot show that (a refusal moved below the reset prints the same line
        after the reset ran), so the seeded stamp and binary must survive. The control in `ImpliedPlatform`
        shows that a building goal does reset them."""
        for goal in ("all", "sim-macos", "print-appid"):
            with self.subTest(goal=goal):
                self.seed()
                proc = make("PLATFORM=bogus", goal, cwd=self.tree, dry=True)
                self.assertNotEqual(proc.returncode, 0)
                self.assertIn('unknown PLATFORM "bogus"', proc.stderr)
                self.assert_untouched(f"make PLATFORM=bogus {goal}")


def stdlib_names() -> frozenset[str]:
    return frozenset(getattr(sys, "stdlib_module_names", ())) | {"platform"}


def imports_platform(path: Path) -> bool:
    tree = ast.parse(path.read_text())
    return any(isinstance(node, ast.Import) and any(alias.name == "platform" for alias in node.names)
               for node in ast.walk(tree))


def consumers() -> list[Path]:
    """The scripts in ci/ that import the standard library's `platform`."""
    return sorted(p for p in CI.glob("*.py") if p != Path(__file__).resolve() and imports_platform(p))


# Imports a file the way a script run from its own directory sees its imports: that directory first on
# `sys.path`. Prints where `platform` came from and what `platform.system()` answered.
PROBE = textwrap.dedent("""\
    import importlib.util, pathlib, sys
    here = pathlib.Path(sys.argv[1])
    sys.path.insert(0, str(here))
    if len(sys.argv) > 2:
        spec = importlib.util.spec_from_file_location("consumer", sys.argv[2])
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        platform = module.platform
    else:
        import platform
    print(platform.__file__)
    print(platform.system() != "" and platform.machine() != "")
    """)


class StandardLibraryIsNotShadowed(unittest.TestCase):
    def probe(self, directory: Path, consumer: Path | None = None) -> tuple[Path, str]:
        args = [sys.executable, "-B", "-c", PROBE, str(directory)] + ([str(consumer)] if consumer else [])
        proc = subprocess.run(args, capture_output=True, text=True, timeout=60, cwd=directory)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        where, works = proc.stdout.splitlines()
        return Path(where).resolve(), works

    def test_there_are_consumers_to_protect(self):
        self.assertGreaterEqual({p.name for p in consumers()},
                                {"mkmacapp.py", "build-libass.py", "test-compat.py"})

    def test_nothing_in_ci_is_named_like_a_standard_library_module(self):
        shadows = sorted(p.name for p in CI.iterdir()
                         if (p.suffix == ".py" or (p / "__init__.py").exists()) and p.stem in stdlib_names())
        self.assertEqual(shadows, [], "a module in ci/ shadows the standard library for every script run from ci/")

    def test_a_probe_run_from_ci_gets_the_real_platform(self):
        where, works = self.probe(CI)
        self.assertNotEqual(where.parent, CI.resolve())
        self.assertEqual((where.name, works), ("platform.py", "True"))

    def test_every_existing_consumer_imports_the_real_platform_from_ci(self):
        for script in consumers():
            with self.subTest(script=script.name):
                where, works = self.probe(CI, script)
                self.assertNotEqual(where.parent, CI.resolve())
                self.assertEqual((where.name, works), ("platform.py", "True"))

    def test_the_consumers_that_parse_arguments_still_start_from_ci(self):
        for script in ("test-compat.py", "mkmacapp.py"):
            with self.subTest(script=script):
                proc = subprocess.run([sys.executable, script, "--help"], capture_output=True, text=True,
                                      timeout=60, cwd=CI)
                self.assertEqual(proc.returncode, 0, proc.stderr)
                self.assertIn("usage:", proc.stdout.lower())

    def test_the_probe_notices_a_shadow(self):
        """The control: with a `platform.py` beside the script the same probe must report that file, or the
        tests above would pass whatever ci/ held."""
        with tempfile.TemporaryDirectory(prefix="plx-shadow.") as td:
            fake = Path(td)
            (fake / "platform.py").write_text('def system():\n    return "shadow"\n\ndef machine():\n    return "shadow"\n')
            consumer = fake / "consumer.py"
            consumer.write_text("import platform\n")
            for use in (None, consumer):
                with self.subTest(consumer=bool(use)):
                    where, _ = self.probe(fake, use)
                    self.assertEqual(where, (fake / "platform.py").resolve())


if __name__ == "__main__":
    unittest.main()
