# Projects and files

## The project

A project is a directory with a `wirebug.toml` manifest at its root and
`main.wb`, the entry file, beside it. Every command finds the project the
way Cargo does: it walks up from the current directory to the first
`wirebug.toml`, or takes the project directory or the manifest as an
argument.

```text
my-project/
├── wirebug.toml
├── main.wb
└── components/
    ├── battery.wb
    └── inverter.wb
```

Only `main.wb` has a fixed name. The other files are loaded because a
`use` or `fragment` statement names them, starting from `main.wb`; a `.wb`
file nothing names is not part of the project. Directories are for the
reader: moving a file changes nothing but the paths that name it.

`main.wb` defines exactly one top-level component, the **root** of the
design (`no_root` otherwise). Everything else is placed inside it,
directly or through the components it places.

## The manifest

`wirebug.toml` holds one `[project]` table. `name` and `version` are
required, the other keys optional, and any key not listed here is an
error.

```toml
[project]
name        = "my-project"
version     = "0.1.0"
description = "Wiring of the test rig"
authors     = ["A. Builder"]
license     = "MIT"
revision    = "B"
date        = "2026-05-28"
```

The rendered drawings carry a stamp of `name`, `version`, `revision` and
`date`. When `revision` is left out it is taken from git (the short commit,
with `-dirty` when the working tree has changes).

## What a file holds

A `.wb` file holds, in any order:

- `use` and `fragment` statements, which name other files;
- `component` definitions and `extend`s of them;
- `connector_type` definitions;
- `view`s.

```wb
use Battery from "components/battery.wb";
fragment "systems/lighting.wb";

connector_type Dt04_2p "Deutsch DT04-2P" {
    part: "DT04-2P";
}

component Vehicle {
    pack: Battery "Battery";
}

view schematic "Overview" {
    include pack at (4, 4);
}
```

## `use`

```wb
use Inverter from "components/inverter.wb";
use Dt06_2s  from "components/connectors.wb";
```

`use <Name> from "<path>";` loads the file at `path`, relative to the file
the statement is in, and brings its top-level component or connector type
`Name` into scope. One statement imports one name: to import two names from
one file, write two statements. Only top-level definitions can be
imported; a component nested in another is private to it.

A path that names no file is `use_not_found`, a name the file does not
define `unresolved_import`, and a name the file never uses
`unused_import` (a warning).

A file loads once however many statements name it, so two files may name
each other.

## `fragment`

```wb
fragment "systems/traction.wb";
```

`fragment "<path>";` loads a file without bringing any name into scope. It
is how a file of [`extend`](#extend)s and views joins the project: each
top-level `extend` in it merges into the component of its name that is in
scope where the `fragment` line is written, and its views are drawn. A
component the file defines is not in scope by it; `use` it to place it.

A `fragment` line that adds nothing is the `unused_fragment` warning, its
help saying why; naming one file twice from one file is
`duplicate_fragment`.

## `extend`

A large component can be written across several files: one file defines
it with `component`, the others add to it with `extend`. The parts merge
into one component, and every name declared in one part (an instance, a
port, a splice) can be used in the others.

```wb
// main.wb
use Battery from "components/battery.wb";
fragment "systems/traction.wb";

component Vehicle {
    pack: Battery "Battery";
}
```

```wb
// systems/traction.wb
use Inverter from "../components/inverter.wb";

extend Vehicle {
    inv: Inverter "Inverter";

    wire orange 50 "HV+" [pack.hv_pos, inv.dc_pos];   // `pack` is in main.wb
    wire orange 50 "HV-" [pack.hv_neg, inv.dc_neg];
}

view schematic "Traction" {
    include pack at (2, 4) ports {
        east: hv_pos, hv_neg;
    }

    include inv at (14, 4) ports {
        west: dc_pos, dc_neg;
    }
}
```

- `extend` is top-level only (`nested_extend` otherwise), and every
  `extend <Name>` needs a `component <Name>` in scope where its file is
  pulled in (`orphan_fragment` otherwise, its help naming the `use` to
  add).
- Each part brings its own `use`s for the types it places.
- A file of `fragment` lines that pulls in further files of `extend`s needs
  the component in its own scope: an `extend` of its own, or
  `use Vehicle from "../main.wb";`.
- [Part data](components.md#part-data) is set once across all the parts.
- A view in a file of `extend`s documents the extended component.

`use Vehicle from "systems/traction.wb";`, naming the extended component,
is an older spelling of `fragment "systems/traction.wb";` and still works;
prefer `fragment`, which says what it does.
