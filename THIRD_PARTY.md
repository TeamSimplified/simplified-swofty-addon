# Third-party notices

## Simplified Suite

This addon reads the `.serps` package format of the Simplified Suite by Team Simplified, <https://github.com/TeamSimplified/simplified-suite>. Its `mod.serps` reader (`src/serps.rs`) and its package root, content root and entry path checks (`src/package.rs`) are ported from `ModManifest.cs`, `ModImport.cs` and `ModStaging.cs` of the Simplified Launcher, with Team Simplified's permission. Its texture array slice merge (`src/texarray.rs`) is ported from `merge_resources.py` of the Simplified Launcher and `LogoBaker.cs` and `LogoSource.cs` of the Simplified Team Editor. The sidebar mark (`icon.svg`, `sections/mods.svg`) is Team Simplified's. So are the archives under `base/`, from their SERPs Base Files for F1 25 v4.1. The Simplified Suite is licensed under the GNU General Public License version 3, which section 13 lets this AGPL-3.0-or-later program combine with.

Based on Simplified Suite by Team Simplified - https://github.com/TeamSimplified/simplified-suite

```
Simplified Suite
Copyright (C) 2026 Team Simplified

This program is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, version 3 of the License. It is distributed WITHOUT ANY WARRANTY;
see LICENSE for the full terms.

Under section 7(b) of the GNU General Public License version 3, the following
attribution requirement applies to this work and to every work based on it:

> Any program or library that contains, copies, ports or is based on code from
> Simplified Suite must credit it, by preserving this notice in its source and
> by stating, in its credits, About screen or documentation - wherever the
> program shows its own Appropriate Legal Notices:
>
> **Based on Simplified Suite by Team Simplified - https://github.com/TeamSimplified/simplified-suite**
```
