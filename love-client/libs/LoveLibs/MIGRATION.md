# Migrating Projects to LoveLibs

This guide explains how to migrate Emily, Character, and Fishbowl to use LoveLibs as a shared git subtree.

## Prerequisites

1. LoveLibs must be its own git repository
2. Each project (Emily, Character, Fishbowl) must be a git repository

---

## Step 1: Initialize LoveLibs as a Git Repository

```bash
cd C:\Users\chris\dev\LoveLibs
git init
git add .
git commit -m "Initial commit: consolidated Love2D libraries"
```

### Optional: Push to Remote

If you want to host LoveLibs on GitHub/GitLab:

```bash
git remote add origin https://github.com/yourusername/LoveLibs.git
git branch -M main
git push -u origin main
```

---

## Step 2: Add LoveLibs as a Subtree to Each Project

### For Emily

```bash
cd C:\Users\chris\dev\Emily

# Add the subtree (use local path or remote URL)
git subtree add --prefix=LoveLibs C:\Users\chris\dev\LoveLibs main --squash

# Or if using a remote repository:
# git subtree add --prefix=LoveLibs https://github.com/yourusername/LoveLibs.git main --squash
```

### For Character

```bash
cd C:\Users\chris\dev\Character
git subtree add --prefix=LoveLibs C:\Users\chris\dev\LoveLibs main --squash
```

### For Fishbowl

```bash
cd C:\Users\chris\dev\Fishbowl
git subtree add --prefix=LoveLibs C:\Users\chris\dev\LoveLibs main --squash
```

---

## Step 3: Update Require Paths

Each project needs its require statements updated to point to the new location.

### Emily

| Old Path | New Path |
|----------|----------|
| `require("libs.scaling")` | `require("LoveLibs.graphics.scaling")` |
| `require("libs.input")` | `require("LoveLibs.core.input")` |
| `require("libs.scenes")` | `require("LoveLibs.core.scenes")` |
| `require("libs.settings")` | `require("LoveLibs.core.settings")` |
| `require("libs.debug")` | `require("LoveLibs.core.debug")` |
| `require("libs.audio")` | `require("LoveLibs.audio.sfx")` |
| `require("libs.music")` | `require("LoveLibs.audio.music")` |

### Character

| Old Path | New Path |
|----------|----------|
| `require("lib.vec2")` | `require("LoveLibs.physics.vec2")` |
| `require("lib.verlet")` | `require("LoveLibs.physics.verlet")` |
| `require("lib.spring")` | `require("LoveLibs.physics.spring")` |
| `require("lib.synth")` | `require("LoveLibs.audio.synth")` |

### Fishbowl

| Old Path | New Path |
|----------|----------|
| `require("lib.scaling")` | `require("LoveLibs.graphics.scaling")` |
| `require("lib.postfx")` | `require("LoveLibs.graphics.postfx")` |
| `require("lib.particles")` | `require("LoveLibs.graphics.particles")` |

### Search and Replace Commands

You can use these commands to find all require statements that need updating:

```bash
# Emily
grep -rn "require.*libs\." C:\Users\chris\dev\Emily --include="*.lua"

# Character
grep -rn "require.*lib\." C:\Users\chris\dev\Character --include="*.lua"

# Fishbowl
grep -rn "require.*lib\." C:\Users\chris\dev\Fishbowl --include="*.lua"
```

---

## Step 4: Remove Old Library Folders

After updating all require paths and verifying everything works:

### Emily

```bash
cd C:\Users\chris\dev\Emily
rm -rf libs/
git add -A
git commit -m "Remove local libs, now using LoveLibs subtree"
```

### Character

```bash
cd C:\Users\chris\dev\Character
rm -rf lib/
git add -A
git commit -m "Remove local lib, now using LoveLibs subtree"
```

### Fishbowl

```bash
cd C:\Users\chris\dev\Fishbowl
# Note: Fishbowl uses C:\Users\chris\dev\lib which is shared
# Only remove if no other projects use it
```

---

## Step 5: Verify Each Project

Test each project to ensure it runs correctly:

```bash
cd C:\Users\chris\dev\Emily && love .
cd C:\Users\chris\dev\Character && love .
cd C:\Users\chris\dev\Fishbowl && love .
```

---

## Updating LoveLibs in Projects

When you make changes to LoveLibs and want to pull them into projects:

### Pull Updates from LoveLibs

```bash
cd C:\Users\chris\dev\Emily
git subtree pull --prefix=LoveLibs C:\Users\chris\dev\LoveLibs main --squash
```

### Push Changes from a Project Back to LoveLibs

If you modify LoveLibs within a project and want to push changes back:

```bash
cd C:\Users\chris\dev\Emily
git subtree push --prefix=LoveLibs C:\Users\chris\dev\LoveLibs main
```

---

## Alternative: Using Git Submodules

If you prefer submodules over subtrees:

```bash
cd C:\Users\chris\dev\Emily
git submodule add https://github.com/yourusername/LoveLibs.git LoveLibs
git commit -m "Add LoveLibs as submodule"
```

**Subtree vs Submodule:**
- **Subtree**: Code is copied into your repo, simpler workflow, no extra clone steps
- **Submodule**: Reference to another repo, requires `git submodule update --init` after clone

---

## Troubleshooting

### "module not found" errors

1. Check that the LoveLibs folder exists in the project root
2. Verify the require path matches the folder structure exactly
3. Ensure LOVE's require path includes the project root

### Scaling API differences

The merged scaling library supports both APIs:
- Emily style: `scaling.start()`, `scaling.stop()`, `scaling.toVirtual(x, y)`
- Fishbowl style: `scaling.apply()`, `scaling.reset()`, `scaling.toGame(x, y)`

Both work interchangeably.

### Physics require path issues

The verlet.lua and spring.lua files try multiple paths to find vec2:
1. `LoveLibs.physics.vec2`
2. `physics.vec2`
3. `lib.vec2`

If none work, ensure vec2.lua is in the expected location.
