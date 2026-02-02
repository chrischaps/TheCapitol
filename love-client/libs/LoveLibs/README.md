# LoveLibs

A collection of reusable libraries for LOVE 2D game development, consolidated from multiple projects.

## Installation

Add LoveLibs to your project via git subtree:

```bash
git subtree add --prefix=LoveLibs https://github.com/your-repo/LoveLibs.git main --squash
```

Or simply copy the folders you need into your project.

## Libraries

### Graphics

| Library | Description |
|---------|-------------|
| `scaling` | Resolution-independent rendering with letterboxing |
| `postfx` | Post-processing effects (bloom, distortion, color grading) |
| `particles` | Ambient floating particle system with light ray interaction |

### Audio

| Library | Description |
|---------|-------------|
| `synth` | Low-level procedural audio synthesis (waveforms, ADSR) |
| `sfx` | High-level game sounds (20+ presets, footsteps, UI sounds) |
| `music` | Procedural ambient music generation |

### Physics

| Library | Description |
|---------|-------------|
| `vec2` | 2D vector math with metamethod operators |
| `verlet` | Verlet physics for ropes, chains, and soft bodies |
| `spring` | Spring-damper physics using Hooke's law |

### Core

| Library | Description |
|---------|-------------|
| `input` | Unified keyboard/gamepad input with action mapping |
| `scenes` | Scene lifecycle management and transitions |
| `settings` | Persistent key-value storage for game preferences |
| `debug` | Debug overlay with FPS, memory, and custom metrics |

---

## Quick Examples

### Scaling

```lua
local scaling = require("LoveLibs.graphics.scaling")

function love.load()
    scaling.init(960, 540)  -- Design resolution
end

function love.resize(w, h)
    scaling.resize(w, h)
end

function love.draw()
    scaling.start()
    -- Draw at virtual resolution
    love.graphics.circle("fill", 480, 270, 50)
    scaling.stop()
end

function love.mousepressed(x, y, button)
    local vx, vy = scaling.toVirtual(x, y)
    -- Use virtual coordinates
end
```

### Physics (Verlet Chain)

```lua
local Verlet = require("LoveLibs.physics.verlet")

local chain = Verlet.Chain(100, 50, 10, 15)  -- x, y, segments, segment_length
chain:get_head().pinned = true

function love.update(dt)
    chain:update(dt, {x = 0, y = 980})
end

function love.draw()
    chain:draw_smooth(4, {0.8, 0.6, 0.4})
end
```

### Audio Synthesis

```lua
local synth = require("LoveLibs.audio.synth")
local s = synth.new()

-- Use presets
local jump = s:create_sound('jump')
jump:play()

-- Or generate custom sounds
local custom = s:generate({
    duration = 0.2,
    frequency = {start = 440, finish = 880},
    waveform = 'sine',
    envelope = {attack = 0.01, decay = 0.1, sustain = 0.5, release = 0.09}
})
```

### Game Sound Effects

```lua
local sfx = require("LoveLibs.audio.sfx")

function love.load()
    sfx.init()
end

function love.update(dt)
    sfx.updateFootsteps(dt, player.isMoving, player.speed)
end

function onButtonClick()
    sfx.play("ui_select")
end
```

### Input System

```lua
local input = require("LoveLibs.core.input")

function love.update(dt)
    local mx, my, isAnalog = input.getMovement()
    player.x = player.x + mx * speed * dt
    player.y = player.y + my * speed * dt

    if input.isPressed("confirm") then
        -- Handle confirm
    end
end
```

### Scene Management

```lua
local scenes = require("LoveLibs.core.scenes")

function love.load()
    scenes.register("menu", require("scenes.menu"))
    scenes.register("game", require("scenes.game"))
    scenes.switchTo("menu")
end

function love.update(dt) scenes.update(dt) end
function love.draw() scenes.draw() end
function love.keypressed(...) scenes.keypressed(...) end
```

---

## Library Details

### graphics/scaling.lua

Resolution-independent rendering with automatic letterboxing.

**Key Functions:**
- `init(width, height)` - Set design resolution
- `start()` / `stop()` - Wrap drawing code
- `toVirtual(x, y)` - Convert screen to virtual coords
- `toScreen(x, y)` - Convert virtual to screen coords
- `getScale()` - Get current scale factor
- `drawBars(r, g, b)` - Draw colored letterbox bars

### graphics/postfx.lua

Multi-pass shader pipeline for visual effects.

**Effects:**
- Water/wave distortion
- Bloom with configurable threshold
- Color grading (saturation, contrast, warmth)
- Vignette
- Dithering

**Key Functions:**
- `init(w, h, config)` - Initialize pipeline
- `beginScene()` / `endScene(scale, offsetX, offsetY)` - Wrap drawing
- `update(dt)` - Update time uniforms
- `handleKeyPress(key)` - F1-F4 toggle effects

### graphics/particles.lua

Ambient floating particle system with depth layers.

**Features:**
- Back/front layer separation for depth
- Light ray interaction (particles glow in light)
- Configurable colors, sizes, speeds
- Burst spawning for effects

### physics/vec2.lua

Complete 2D vector math with operator overloading.

```lua
local a = vec2.new(10, 20)
local b = vec2.new(5, 5)
local c = a + b              -- Metamethod
local d = vec2.normalize(c)  -- Static function
local len = vec2.length(d)
```

### physics/verlet.lua

Position-based physics for stable soft body simulation.

**Classes:**
- `VerletPoint` - Single physics point with mass, friction
- `VerletChain` - Connected points with distance constraints

**Features:**
- Gravity and force application
- Ground/bounds constraints
- Catmull-Rom spline rendering
- Debug visualization

### physics/spring.lua

Spring-damper connections using Hooke's law.

**Features:**
- Configurable stiffness and damping
- Color-coded stretch visualization
- SpringSystem for managing multiple springs

### audio/synth.lua

Low-level procedural audio synthesis.

**Waveforms:** sine, saw, square, triangle, noise, pulse

**Presets:** jump, land, hit, pickup, coin, laser, explosion, powerup, hurt, blip

### audio/sfx.lua

High-level game audio with presets.

**Built-in Sounds:**
- Chords: success, partial, reject, stabilize, destabilize
- UI: blip, ui_navigate, ui_select, ui_back, ui_adjust
- Impacts: click, thunk, reward_pop, reward_slam
- Ambient: drone, drumroll, ending
- Movement: footstep1-4

### audio/music.lua

Procedural ambient music generation.

**Styles:** fog, calm, tense

**Features:**
- Multi-layer drone synthesis
- Fade in/out
- Dynamic modulation

### core/input.lua

Unified input for keyboard and gamepad.

**Features:**
- Analog stick with deadzone
- Action mapping system
- 4-way and 8-way direction detection

### core/scenes.lua

Scene lifecycle management.

**Features:**
- Scene registration and switching
- Push/pop scene stack for modals
- Automatic LOVE callback delegation

### core/settings.lua

Persistent key-value storage.

**Features:**
- Schema-based with type coercion
- Change callbacks
- Convenience methods (toggle, cycle, increment)

### core/debug.lua

Debug overlay system.

**Features:**
- FPS and memory display
- Custom debug info via `getDebugInfo()`
- Progress bar visualization
- Point/rect/line drawing utilities

---

## Origins

Libraries consolidated from:
- **Emily** - Educational emotion game (input, scenes, settings, debug, music, sfx, scaling)
- **Character** - Procedural animation project (vec2, verlet, spring, synth)
- **Fishbowl** - Aquarium simulation (postfx, particles, scaling)

## License

MIT License - See individual library headers for details.
