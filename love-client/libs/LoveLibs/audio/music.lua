--[[
    love2d-music
    Procedural ambient music system for LOVE 2D

    Generates evolving background drones and atmospheric soundscapes
    entirely through synthesis - no audio files required.

    USAGE:
        local music = require("libs.music")

        function love.load()
            music.play("calm")  -- Start ambient music
        end

        function love.update(dt)
            music.update(dt)  -- Required for fading

            -- Optional: modulate based on game state
            music.modulate(tensionLevel)  -- 0-1
        end

        function onGameEnd()
            music.fadeOut(2.0)  -- Fade out over 2 seconds
        end

    BUILT-IN STYLES:
        "fog" / "disorientation" - Mysterious, unsettling ambient
        "calm" - Peaceful, grounding tones
        "tense" - Dissonant, anxiety-inducing

    API:
        music.play(style)
            Start playing ambient music with a style preset

        music.stop()
            Stop all music immediately

        music.fadeOut(duration)
            Fade out music over duration seconds

        music.setVolume(volume)
            Set master music volume (0-1)

        music.getVolume() -> number
            Get master music volume

        music.isPlaying() -> boolean
            Check if music is currently playing

        music.update(dt)
            Update music system (required for fading)

        music.modulate(factor)
            Dynamically modulate intensity (0-1, higher = more intense)

        music.addStyle(name, config)
            Add a custom style preset

        music.generateDrone(baseFreq, duration, volume, detune) -> source
            Generate a drone layer for custom compositions

        music.generatePad(baseFreq, duration, volume) -> source
            Generate a pad layer for custom compositions

    LICENSE: MIT
]]

local music = {}

local activeSources = {}
local masterVolume = 0.45
local playing = false

-- Fade state
local fading = {
    active = false,
    duration = 2.0,
    elapsed = 0,
    startVolume = 0.45
}

-- Style presets
local styles = {}

--------------------------------------------------
-- SOUND GENERATION
--------------------------------------------------

--- Generate an ambient drone layer
-- @param baseFreq Base frequency in Hz
-- @param duration Duration in seconds (loops)
-- @param volume Volume (0-1)
-- @param detune Optional detuning factor
-- @return LOVE audio source
function music.generateDrone(baseFreq, duration, volume, detune)
    local sampleRate = 44100
    local samples = math.floor(sampleRate * duration)
    local soundData = love.sound.newSoundData(samples, sampleRate, 16, 1)

    detune = detune or 0

    -- Frequencies for rich harmonic content
    local freq1 = baseFreq * (1 + detune * 0.01)
    local freq2 = baseFreq * 2.01  -- Slight detuning for movement
    local freq3 = baseFreq * 3.005
    local freq4 = baseFreq * 0.5

    for i = 0, samples - 1 do
        local t = i / sampleRate

        -- Slow amplitude modulation for movement
        local ampMod = 0.85 + 0.15 * math.sin(t * 0.3)

        -- Fade in/out for seamless looping
        local envelope = 1.0
        local fadeTime = duration * 0.1
        if t < fadeTime then
            envelope = t / fadeTime
        elseif t > duration - fadeTime then
            envelope = (duration - t) / fadeTime
        end

        -- Layer multiple harmonics
        local sample = 0
        sample = sample + math.sin(2 * math.pi * freq1 * t) * 0.4
        sample = sample + math.sin(2 * math.pi * freq2 * t) * 0.2
        sample = sample + math.sin(2 * math.pi * freq3 * t) * 0.1
        sample = sample + math.sin(2 * math.pi * freq4 * t) * 0.3

        -- Add subtle noise texture
        local noiseAmount = 0.02
        sample = sample + (math.random() * 2 - 1) * noiseAmount

        soundData:setSample(i, sample * volume * envelope * ampMod)
    end

    local source = love.audio.newSource(soundData)
    source:setLooping(true)
    return source
end

--- Generate a pad layer with filter-like sweep
-- @param baseFreq Base frequency in Hz
-- @param duration Duration in seconds (loops)
-- @param volume Volume (0-1)
-- @return LOVE audio source
function music.generatePad(baseFreq, duration, volume)
    local sampleRate = 44100
    local samples = math.floor(sampleRate * duration)
    local soundData = love.sound.newSoundData(samples, sampleRate, 16, 1)

    for i = 0, samples - 1 do
        local t = i / sampleRate

        -- Slow harmonic sweep
        local sweepPhase = math.sin(t * 0.1) * 0.5 + 0.5
        local harmonicMix = sweepPhase

        -- Fade envelope
        local envelope = 1.0
        local fadeTime = duration * 0.15
        if t < fadeTime then
            envelope = t / fadeTime
        elseif t > duration - fadeTime then
            envelope = (duration - t) / fadeTime
        end

        -- Triangle wave with varying harmonics
        local sample = 0
        local phase1 = (t * baseFreq) % 1
        local phase2 = (t * baseFreq * 1.5) % 1
        local phase3 = (t * baseFreq * 2) % 1

        -- Triangle waves
        sample = sample + (4 * math.abs(phase1 - 0.5) - 1) * 0.5
        sample = sample + (4 * math.abs(phase2 - 0.5) - 1) * 0.2 * harmonicMix
        sample = sample + (4 * math.abs(phase3 - 0.5) - 1) * 0.15 * (1 - harmonicMix)

        soundData:setSample(i, sample * volume * envelope)
    end

    local source = love.audio.newSource(soundData)
    source:setLooping(true)
    return source
end

--------------------------------------------------
-- STYLE PRESETS
--------------------------------------------------

-- Initialize default styles
local function initStyles()
    styles.fog = {
        { generator = "drone", baseFreq = 55, duration = 8, volume = 0.25, detune = 0 },
        { generator = "drone", baseFreq = 82.5, duration = 10, volume = 0.15, detune = 2 },
        { generator = "pad", baseFreq = 110, duration = 12, volume = 0.12 }
    }
    styles.disorientation = styles.fog  -- Alias

    styles.calm = {
        { generator = "drone", baseFreq = 65.41, duration = 10, volume = 0.2, detune = 0 },
        { generator = "drone", baseFreq = 98, duration = 12, volume = 0.15, detune = 1 },
        { generator = "pad", baseFreq = 130.81, duration = 14, volume = 0.1 }
    }

    styles.tense = {
        { generator = "drone", baseFreq = 58.27, duration = 8, volume = 0.2, detune = 3 },
        { generator = "drone", baseFreq = 61.74, duration = 10, volume = 0.18, detune = -2 },
        { generator = "pad", baseFreq = 116.54, duration = 12, volume = 0.1 }
    }
end

-- Initialize layers from a style config
local function initLayers(style)
    music.stop()
    activeSources = {}

    local config = styles[style]
    if not config then
        config = styles.fog  -- Default fallback
    end

    for _, layerConfig in ipairs(config) do
        local source
        if layerConfig.generator == "drone" then
            source = music.generateDrone(
                layerConfig.baseFreq,
                layerConfig.duration,
                layerConfig.volume,
                layerConfig.detune
            )
        elseif layerConfig.generator == "pad" then
            source = music.generatePad(
                layerConfig.baseFreq,
                layerConfig.duration,
                layerConfig.volume
            )
        end

        if source then
            table.insert(activeSources, {
                source = source,
                baseVolume = layerConfig.volume
            })
        end
    end
end

--------------------------------------------------
-- PUBLIC API
--------------------------------------------------

--- Start playing ambient music
-- @param style Style preset name (default "fog")
function music.play(style)
    if not next(styles) then initStyles() end

    style = style or "fog"
    initLayers(style)

    for _, layer in ipairs(activeSources) do
        layer.source:setVolume(layer.baseVolume * masterVolume)
        layer.source:play()
    end

    playing = true
    fading.active = false
end

--- Stop all music immediately
function music.stop()
    for _, layer in ipairs(activeSources) do
        if layer.source then
            layer.source:stop()
        end
    end
    playing = false
    fading.active = false
end

--- Fade out music over duration
-- @param duration Fade duration in seconds (default 2)
function music.fadeOut(duration)
    fading = {
        active = true,
        duration = duration or 2.0,
        elapsed = 0,
        startVolume = masterVolume
    }
end

--- Update music system (required for fading)
-- @param dt Delta time
function music.update(dt)
    if fading.active then
        fading.elapsed = fading.elapsed + dt
        local progress = fading.elapsed / fading.duration

        if progress >= 1 then
            music.stop()
            masterVolume = fading.startVolume
        else
            local currentVolume = fading.startVolume * (1 - progress)
            for _, layer in ipairs(activeSources) do
                layer.source:setVolume(layer.baseVolume * currentVolume)
            end
        end
    end
end

--- Set master volume
-- @param vol Volume (0-1)
function music.setVolume(vol)
    masterVolume = math.max(0, math.min(1, vol))
    for _, layer in ipairs(activeSources) do
        layer.source:setVolume(layer.baseVolume * masterVolume)
    end
end

--- Get current volume
-- @return number
function music.getVolume()
    return masterVolume
end

--- Check if music is playing
-- @return boolean
function music.isPlaying()
    return playing
end

--- Modulate music based on game state
-- @param factor Intensity factor (0-1, higher = more intense)
function music.modulate(factor)
    for i, layer in ipairs(activeSources) do
        local modVolume = layer.baseVolume * masterVolume
        -- Higher layers get louder with higher factor
        if i > 1 then
            modVolume = modVolume * (0.7 + factor * 0.6)
        end
        layer.source:setVolume(modVolume)
    end
end

--- Add a custom style preset
-- @param name Style name
-- @param config Table of layer configurations
function music.addStyle(name, config)
    if not next(styles) then initStyles() end
    styles[name] = config
end

--- Remove a style preset
-- @param name Style name
function music.removeStyle(name)
    styles[name] = nil
end

--- Get available style names
-- @return table
function music.getStyles()
    if not next(styles) then initStyles() end
    local list = {}
    for name, _ in pairs(styles) do
        table.insert(list, name)
    end
    return list
end

--- Check if a style exists
-- @param name Style name
-- @return boolean
function music.hasStyle(name)
    if not next(styles) then initStyles() end
    return styles[name] ~= nil
end

return music
