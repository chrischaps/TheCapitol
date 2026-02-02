# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

LoveLibs is a reusable game development library collection for LÖVE 2D (Lua-based 2D game framework). This is a consolidation of libraries from three game projects (Emily, Character, Fishbowl) into one unified, modular collection.

**Language**: Lua 5.1+
**Framework**: LÖVE 2D
**No build process** - pure Lua modules requiring no compilation

## Running Projects

```bash
# Run LÖVE 2D games that integrate this library
love .

# Run specific integrated projects
cd C:\Users\chris\dev\Emily && love .
cd C:\Users\chris\dev\Character && love .
cd C:\Users\chris\dev\Fishbowl && love .
```

## Architecture

### Module Pattern
All Lua files follow the table-based module pattern:
```lua
local module = {}
-- ... functions ...
return module
```

### Library Categories

**core/** - Game systems
- `input.lua` - Unified keyboard/gamepad input with action mapping
- `scenes.lua` - Scene lifecycle management with stack support
- `settings.lua` - Schema-based persistent settings with type coercion
- `debug.lua` - F3-toggleable debug overlay with FPS/memory monitoring

**graphics/** - Visual systems
- `scaling.lua` - Resolution-independent rendering with letterboxing
- `postfx.lua` - Multi-pass shader pipeline (bloom, distortion, color grading)
- `particles.lua` - Depth-layered ambient particle system

**audio/** - Sound systems
- `synth.lua` - Low-level procedural synthesis (waveforms, ADSR envelopes)
- `sfx.lua` - High-level game sound effects (20+ built-in sounds)
- `music.lua` - Procedural ambient music generation

**physics/** - Simulation
- `vec2.lua` - 2D vector math with metamethod operators (+, -, *, /)
- `verlet.lua` - Position-based physics for chains/cloth
- `spring.lua` - Hooke's law spring-damper physics

### Key Patterns

**Settings schema with defaults and callbacks**:
```lua
Settings.init({
    volume = { default = 1.0, type = "number" },
    fullscreen = { default = false, type = "boolean" }
})
```

**Scene registration and transitions**:
```lua
Scenes.register("game", GameScene)
Scenes.switch("game")
```

**Scaling coordinate transformation**:
```lua
Scaling.start()  -- Begin scaled rendering
-- draw calls here use virtual coordinates
Scaling.stop()
local vx, vy = Scaling.toVirtual(mx, my)  -- Transform mouse input
```

### Consolidation Notes

Some modules support dual APIs for backwards compatibility with original projects:
- `scaling.lua`: Both `start/stop` and `apply/finish` APIs
- `scaling.lua`: Both `toVirtual/toScreen` and `toGame/toWorld` naming

## Design Principles

1. **No external dependencies** - only uses LÖVE framework features
2. **Modular** - each library is independent and self-contained
3. **Composable** - libraries work together but don't require each other
4. **Documented** - extensive inline documentation in every module

## Integration Method

Libraries are integrated into game projects as a git subtree at `libs/`:
```bash
git subtree add --prefix libs https://github.com/user/LoveLibs.git main --squash
```
