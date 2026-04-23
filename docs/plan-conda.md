# Implementation plan: conda backend for UniPack

This document plans a **`conda`** tab for listing packages in a **single target environment**, merging **upgrade-available** metadata where feasible, and running **`conda update`** / **`conda remove`** non-interactively. It draws on the **official conda command reference** and **user guide**, and aligns with UniPack’s existing shell-backed backends (`list` / `latest` / `counts` / `commands` in [`src/pkg_manager/`](src/pkg_manager/)).

## References (official)

- **Managing packages** (list, update, remove, pip interop, pinning): [Managing packages — conda documentation](https://docs.conda.io/projects/conda/en/stable/user-guide/tasks/manage-pkgs.html)
- **`conda list`**: [conda list command reference](https://docs.conda.io/projects/conda/en/stable/commands/list.html)
- **`conda update`**: [conda update command reference](https://docs.conda.io/projects/conda/en/stable/commands/update.html)
- **`conda remove`**: [conda remove command reference](https://docs.conda.io/projects/conda/en/stable/commands/remove.html)
- **Command index** (cross-links, other subcommands): [Commands — conda documentation](https://docs.conda.io/projects/conda/en/stable/commands/index.html)
- **Mamba** (optional faster drop-in for many workflows): [Mamba user guide](https://mamba.readthedocs.io/en/latest/user_guide/mamba.html)

Re-check **`conda <cmd> --help`** for the minimum conda version you support; flags such as **`--solver`** evolve.

## Goals

- Detect **`conda`** on `PATH` (optional: prefer **`mamba`** or **`micromamba`** when present and CLI-compatible for the chosen subcommands).
- **List** installed packages in **one** selected environment with **stable JSON**.
- **Upgrade** / **remove** selected packages using conda, with **no interactive prompts** (UniPack runs non-interactively, like apt/pacman paths today).
- **Outdated / upgradable** metadata: respect the **solver** (updates may involve transitive changes); see [Upgrade metadata strategies](#upgrade-metadata-strategies).
- **Installing** new packages remains **out of scope** (same product rule as other backends).

## Non-goals (v1)

- Managing **multiple** conda environments as separate tabs (possible later via env picker or multiple `PackageManager` rows).
- **`conda env create` / `conda install`** (new specs).
- Full **commercial / tokenized** channel auth UX (inherit user’s `.condarc` / env; document limitations).

## How conda differs from pip / apt / raw rpm

### 1. **Environment-centric** model

Every operation is scoped to an **environment** (prefix). Commands accept **`-n NAME`** or **`-p PREFIX`**; otherwise they use the **current** environment (often whatever was last **`conda activate`**d — but UniPack may **not** run inside an activated shell).

**Implication:** Always pass **`-n`** or **`-p`** explicitly after resolving the **target env** (see [Target environment selection](#target-environment-selection)). Do not assume `conda list` without flags matches user intent.

Official: [conda list — Target Environment Specification](https://docs.conda.io/projects/conda/en/stable/commands/list.html#conda.cli.conda_argparse-generate_parser-target-environment-specification).

### 2. **`conda update` is solver-driven, not “bump this row only”**

Official text: conda updates requested packages to the **latest versions compatible with all other packages** in the environment and **may update other installed packages** or **install additional packages** to satisfy dependencies. Options like **`--no-update-deps`** / **`--freeze-installed`** change that trade-off.

**Implication:** UniPack’s “upgrade this row” still maps to **`conda update -y <name>`**, but users should understand **side effects**; surface **stderr** on failure. Optionally document vs **`--freeze-installed`** (stricter, may pick older builds).

Source: [conda update command reference](https://docs.conda.io/projects/conda/en/stable/commands/update.html#conda-update).

### 3. **`conda remove` removes dependents** (unless solver finds replacements)

Official: **`conda remove`** removes packages that **depend** on removed packages unless a replacement exists; **`--force`** removes only the requested packages and **can break the environment**.

**Implication:** Default behaviour matches conda semantics (like DNF dependency removal); do not pass **`--force`** unless adding an explicit advanced / dangerous path.

Source: [conda remove command reference](https://docs.conda.io/projects/conda/en/stable/commands/remove.html#conda-remove).

### 4. **Pip-installed packages inside the env**

**`conda list`** can include packages installed by **pip** into that environment. The list command documents **`--no-pip`**: *“Do not include pip-only installed packages.”*

**Product choice:**

- **v1:** Use **`conda list --json`** **without** `--no-pip` so the UI matches **`conda list`** in a terminal (pip-managed rows appear with conda’s labelling in metadata).
- **Alternative:** **`--no-pip`** for “conda-only” purity; then document that pip-installed tools won’t appear.

Source: [conda list](https://docs.conda.io/projects/conda/en/stable/commands/list.html).

### 5. **Pinning and frozen environments**

User-managed **`conda-meta/pinned`** and conda’s **frozen** protections block or constrain updates; flags such as **`--no-pin`** and **`--override-frozen`** exist (**override-frozen** documented as dangerous).

**Implication:** Default: **do not** pass **`--override-frozen`**. If **`conda update`** fails due to pins/freeze, show stderr; optional future UX: one-line hint (“pinned / frozen — see conda-meta/pinned or env freeze”).

Source: [Managing packages — Preventing packages from updating (pinning)](https://docs.conda.io/projects/conda/en/stable/user-guide/tasks/manage-pkgs.html#preventing-packages-from-updating-pinning); [conda update / conda remove options](https://docs.conda.io/projects/conda/en/stable/commands/update.html).

### 6. **Channels, strict priority, and solvers**

**`conda update` / `conda remove`** accept **`-c` / `--channel`**, **strict-channel-priority**, and **`--solver {classic,libmamba}`** (see command reference).

**Implication:** v1 should **not** override user **`.condarc`** channel configuration unless adding a dedicated setting later; inherit normal conda config by running **`conda`** with the user’s environment.

### 7. **Non-interactive use: `--yes` and `--json`**

**`-y` / `--yes`**: *“Sets any confirmation values to ‘yes’ automatically.”* Required for UniPack’s non-interactive upgrades/removes.

**`--json`**: *“Report all output as json. Suitable for using conda programmatically.”* Use for **`conda list`** parsing; for **`update`/`remove`**, JSON helps distinguish success vs solver errors when combined with exit codes.

**`--dry-run`**: *“Only display what would have been done.”* Useful for building an **upgradable** map without applying transactions (still runs the solver — can be slow).

Sources: [conda update](https://docs.conda.io/projects/conda/en/stable/commands/update.html), [conda remove](https://docs.conda.io/projects/conda/en/stable/commands/remove.html), [conda list](https://docs.conda.io/projects/conda/en/stable/commands/list.html).

### 8. **`mamba` / `micromamba`**

Many users replace or wrap conda with **mamba** for faster solves. Subcommands often mirror **`conda list` / `conda update` / `conda remove`** for core workflows.

**Implication:** Detection order could be: **`mamba`** (if on PATH) → **`conda`** → **`micromamba`**, with one chosen **`command`** string stored in [`PackageManager`](src/pkg_manager/mod.rs) but UI label still **`conda`** or explicit **`mamba`** if you want honesty in the tab name.

## Target environment selection

Without this, the backend is ambiguous.

**Recommended v1 policy:**

1. If **`UNIPACK_CONDA_ENV`** is set to a **non-empty** string, use **`-n <that>`** for all subcommands (document in `README` / `SPEC`).
2. Else if **`CONDA_PREFIX`** is set and points to a valid env, use **`-p "$CONDA_PREFIX"`** (active env when the user launched UniPack from an activated shell).
3. Else resolve **base** prefix from **`conda info --json`** (fields such as **`root_prefix`** / **`conda_prefix`** — validate against your minimum conda; see **`conda info --help`** on your machine) and pass **`-p <base_prefix>`**.

Cache the resolved **`-n` vs `-p`** and prefix string for the process lifetime or until **Ctrl+R** refresh.

## Suggested command mapping (baseline)

Assume resolved target is passed as **either** `-n ENV` **or** `-p PREFIX` (not both).

| Operation | Command sketch | Notes |
|-----------|----------------|--------|
| **Resolve prefix / name** | `conda info --json` | Parse once; handle “conda not initialized” errors clearly. |
| **List** | `conda list -n ENV --json` **or** `conda list -p PREFIX --json` | Primary source for [`Package`](src/model.rs) rows: **name**, **version** from JSON objects (conda emits a JSON array of package dicts — validate against real stdout in tests). |
| **Upgrade** | `conda update -y -n ENV PKG` (or `-p PREFIX`) | Match [conda update](https://docs.conda.io/projects/conda/en/stable/commands/update.html); **`-y`** mandatory. |
| **Remove** | `conda remove -y -n ENV PKG` | Match [conda remove](https://docs.conda.io/projects/conda/en/stable/commands/remove.html). |
| **Optional refresh** | `conda update -y -n ENV --all` is wrong for “metadata only”; prefer **`conda clean`** / index refresh only if you add a dedicated “refresh mirrors” equivalent — conda does not map 1:1 to `pacman -Syy`. v1 can **no-op** `refresh_mirrors_and_upgrade_package` or run **`conda update -y PKG`** only. |

## Upgrade metadata strategies

There is **no** single official “list outdated packages” flag comparable to **`pip list --outdated`**. Practical approaches:

| Strategy | Idea | Pros | Cons |
|----------|------|------|------|
| **A. Env-wide dry-run** | `conda update --all --dry-run --json -y -n ENV` | One solver pass; reflects **real** upgrade set for “update all compatible”. | **Slow**; JSON schema must be parsed carefully (`LINK` / `UNLINK` actions — validate on sample outputs). |
| **B. Per-package dry-run** | `conda update PKG --dry-run --json -y -n ENV` | Narrower diff per row. | **Very slow** for large envs if done per package. |
| **C. List-only v1** | Only **`conda list --json`**; no **`latest_version`** until refresh strategy exists | Fast UI. | **`o`** / counts / overlay less useful until upgraded. |

**Recommendation:** Ship **C** or a **throttled A** (on tab focus / **Ctrl+R** only, with in-memory cache + “stale” indicator). Parsing rules should be covered by **fixture JSON** captured from real conda on Linux.

## Code touchpoints (checklist)

1. **`src/detect.rs`** — Add `("conda", …)` to `PM_CONFIGS`; **`needs_root`**: default **`false`** (user Miniconda/Mambaforge); document edge cases for system-wide installs.
2. **`src/detect.rs`** / small helper — Resolve **target env** (`UNIPACK_CONDA_ENV` / `CONDA_PREFIX` / base from `conda info --json`).
3. **`src/pkg_manager/list.rs`** — `list_conda()` → `conda list --json` with `-n`/`-p`.
4. **`src/pkg_manager/latest.rs`** — `latest_map_conda()` using chosen strategy **A** or **B**; tolerate conda JSON version differences.
5. **`src/pkg_manager/counts.rs`** — `count_conda_updates()` consistent with merged outdated set.
6. **`src/pkg_manager/commands.rs`** — `conda update` / `conda remove` with **`-y`** and same `-n`/`-p`.
7. **`src/pkg_manager/util.rs`** — `ensure_privileges_ready`: conda usually **skipped** unless you detect a root-owned prefix (advanced).
8. **`src/all_upgradables.rs`** / tests — Upgrade/remove use **exact conda package names** from the list JSON (**name** field); watch **namespace / multi-output** packages if they appear with distinct names.
9. **Tests** — Commit **redacted** `conda list --json` samples (small array) + optional dry-run JSON for parser unit tests.
10. **Docs** — `README`, `SPEC`, help text in **`run_loop.rs`** when behaviour is final.

## QA matrix (manual)

- **Base env** only: list, upgrade one leaf package, remove one non-critical package.
- **Named env** via **`UNIPACK_CONDA_ENV`**: same flows.
- **Pinned** package: update attempt fails or no-ops with clear message; no **`--override-frozen`** by default.
- **Mixed pip + conda**: confirm rows appear as expected with default **`conda list --json`** (no `--no-pip`).
- **libmamba solver** (if user has it configured): smoke test that **`conda`** still works unchanged (UniPack does not need to pass **`--solver`** unless you add a toggle).

## Open questions

- Tab label **`conda`** vs **`mamba`** when the binary is **`mamba`**.
- Whether **`conda run -n env …`** should wrap invocations instead of **`-n`** on subcommands (usually equivalent; pick one style for argv consistency).
- How aggressively to cache **dry-run** results vs **Ctrl+R** user expectation (“always re-solve”).

---

*Implementation plan only; after merge, behaviour is authoritative in `SPEC.md` and the code.*
