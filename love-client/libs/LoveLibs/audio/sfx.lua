--[[
    love2d-sfx
    Procedural audio system with sound synthesis for LOVE 2D

    Generates sounds procedurally at runtime - no audio files needed.
    Includes a library of common game sounds plus tools for creating custom sounds.

    USAGE:
        local sfx = require("LoveLibs.audio.sfx")

        function love.load()
            sfx.init()  -- Initialize with default sound library
        end

        function love.update(dt)
            -- Update footsteps based on player movement
            sfx.updateFootsteps(dt, player.isMoving, player.speedFactor)
        end

        function onButtonClick()
            sfx.play("ui_select")
        end

        function onCollect()
            sfx.play("coin", 1.0, 1.2)  -- volume 1.0, pitch 1.2x
        end

    BUILT-IN SOUNDS:
        Chords: success, partial, reject, stabilize, destabilize
        UI: blip, ui_navigate, ui_select, ui_back, ui_adjust
        Impacts: click, thunk, reward_pop, reward_slam
        Ambient: drone, drumroll, ending
        Movement: footstep1, footstep2, footstep3, footstep4

    API:
        sfx.init()
            Initialize the audio system and generate default sounds

        sfx.play(name, volume, pitch)
            Play a sound by name. Volume and pitch are optional multipliers.

        sfx.stop(name)
            Stop a specific sound

        sfx.stopAll()
            Stop all sounds

        sfx.setSfxVolume(volume)
            Set master SFX volume (0-1)

        sfx.getSfxVolume() -> number
            Get master SFX volume

        sfx.has(name) -> boolean
            Check if a sound exists

        sfx.list() -> table
            Get list of available sound names

        sfx.updateFootsteps(dt, isMoving, speedFactor)
            Update footstep sounds based on movement

        sfx.setFootstepParams(interval, volume)
            Configure footstep timing and volume

        sfx.addSound(name, source)
            Add a custom LOVE audio source

        -- Sound generation functions:
        sfx.generateTone(frequency, duration, volume, waveform) -> source
        sfx.generateChord(frequencies, duration, volume, waveform) -> source
        sfx.generateFootstep(pitch, duration, volume) -> source
        sfx.generateNoise(duration, volume, envelope) -> source

    LICENSE: MIT
]]

local sfx = {}

local sounds = {}
local initialized = false

-- Footstep state
local footsteps = {
    timer = 0,
    interval = 0.28,
    lastStep = 0,
    volume = 0.28
}

-- Master SFX volume
local sfxVolume = 0.5

--------------------------------------------------
-- SOUND GENERATION FUNCTIONS
--------------------------------------------------

--- Generate a simple tone using SoundData
-- @param frequency Tone frequency in Hz
-- @param duration Duration in seconds
-- @param volume Volume (0-1)
-- @param waveform "sine", "square", "triangle", or "noise"
-- @return LOVE audio source
function sfx.generateTone(frequency, duration, volume, waveform)
    local sampleRate = 44100
    local samples = math.floor(sampleRate * duration)
    local soundData = love.sound.newSoundData(samples, sampleRate, 16, 1)

    volume = volume or 0.3
    waveform = waveform or "sine"

    for i = 0, samples - 1 do
        local t = i / sampleRate
        local envelope = 1.0

        -- Apply fade out in last 20% of sound
        local fadeStart = duration * 0.8
        if t > fadeStart then
            envelope = 1 - ((t - fadeStart) / (duration - fadeStart))
        end

        -- Apply fade in for first 5%
        local fadeInEnd = duration * 0.05
        if t < fadeInEnd then
            envelope = envelope * (t / fadeInEnd)
        end

        local sample = 0
        if waveform == "sine" then
            sample = math.sin(2 * math.pi * frequency * t)
        elseif waveform == "square" then
            sample = math.sin(2 * math.pi * frequency * t) > 0 and 1 or -1
            sample = sample * 0.5
        elseif waveform == "triangle" then
            local phase = (t * frequency) % 1
            sample = 4 * math.abs(phase - 0.5) - 1
        elseif waveform == "noise" then
            sample = (math.random() * 2 - 1) * 0.5
        end

        soundData:setSample(i, sample * volume * envelope)
    end

    return love.audio.newSource(soundData)
end

--- Generate a chord (multiple frequencies)
-- @param frequencies Table of frequencies
-- @param duration Duration in seconds
-- @param volume Volume (0-1)
-- @param waveform "sine" or "triangle"
-- @return LOVE audio source
function sfx.generateChord(frequencies, duration, volume, waveform)
    local sampleRate = 44100
    local samples = math.floor(sampleRate * duration)
    local soundData = love.sound.newSoundData(samples, sampleRate, 16, 1)

    volume = (volume or 0.3) / #frequencies
    waveform = waveform or "sine"

    for i = 0, samples - 1 do
        local t = i / sampleRate
        local envelope = 1.0

        local fadeStart = duration * 0.7
        if t > fadeStart then
            envelope = 1 - ((t - fadeStart) / (duration - fadeStart))
        end

        local fadeInEnd = duration * 0.05
        if t < fadeInEnd then
            envelope = envelope * (t / fadeInEnd)
        end

        local sample = 0
        for _, freq in ipairs(frequencies) do
            if waveform == "sine" then
                sample = sample + math.sin(2 * math.pi * freq * t)
            elseif waveform == "triangle" then
                local phase = (t * freq) % 1
                sample = sample + (4 * math.abs(phase - 0.5) - 1)
            end
        end

        soundData:setSample(i, sample * volume * envelope)
    end

    return love.audio.newSource(soundData)
end

--- Generate a soft footstep sound
-- @param pitch Base frequency (typically 55-65 Hz)
-- @param duration Duration in seconds
-- @param volume Volume (0-1)
-- @return LOVE audio source
function sfx.generateFootstep(pitch, duration, volume)
    local sampleRate = 44100
    local samples = math.floor(sampleRate * duration)
    local soundData = love.sound.newSoundData(samples, sampleRate, 16, 1)

    volume = volume or 0.1

    for i = 0, samples - 1 do
        local t = i / sampleRate
        local progress = i / samples

        -- Quick attack, fast decay envelope
        local envelope = 0
        if progress < 0.05 then
            envelope = progress / 0.05
        elseif progress < 0.15 then
            envelope = 1 - (progress - 0.05) / 0.1 * 0.6
        else
            envelope = 0.4 * (1 - (progress - 0.15) / 0.85)
        end

        -- Mix of low thump and subtle noise
        local thump = math.sin(2 * math.pi * pitch * t) * 0.6
        local noise = (math.random() * 2 - 1) * 0.4

        local sample = thump + noise * envelope * 0.5

        soundData:setSample(i, sample * volume * envelope)
    end

    return love.audio.newSource(soundData)
end

--- Generate noise with custom envelope
-- @param duration Duration in seconds
-- @param volume Volume (0-1)
-- @param envelope "percussive", "sustain", or "swell"
-- @return LOVE audio source
function sfx.generateNoise(duration, volume, envelope)
    local sampleRate = 44100
    local samples = math.floor(sampleRate * duration)
    local soundData = love.sound.newSoundData(samples, sampleRate, 16, 1)

    volume = volume or 0.3
    envelope = envelope or "percussive"

    for i = 0, samples - 1 do
        local progress = i / samples
        local env = 1.0

        if envelope == "percussive" then
            env = math.exp(-progress * 8)
        elseif envelope == "sustain" then
            if progress < 0.1 then
                env = progress / 0.1
            elseif progress > 0.8 then
                env = (1 - progress) / 0.2
            end
        elseif envelope == "swell" then
            env = math.sin(progress * math.pi)
        end

        local noise = (math.random() * 2 - 1)
        soundData:setSample(i, noise * volume * env)
    end

    return love.audio.newSource(soundData)
end

-- Internal: Generate percussive pop
local function generateRewardPop(duration, volume)
    local sampleRate = 44100
    local samples = math.floor(sampleRate * duration)
    local soundData = love.sound.newSoundData(samples, sampleRate, 16, 1)

    volume = volume or 0.3

    for i = 0, samples - 1 do
        local progress = i / samples
        local envelope = math.exp(-progress * 35)
        local snap = (math.random() * 2 - 1) * math.exp(-progress * 80)
        local body = (math.random() * 2 - 1) * math.exp(-progress * 25)
        local tail = (math.random() * 2 - 1) * math.exp(-progress * 15) * 0.3
        local sample = (snap * 0.5 + body * 0.4 + tail * 0.3) * volume * envelope
        soundData:setSample(i, math.max(-1, math.min(1, sample)))
    end

    return love.audio.newSource(soundData)
end

-- Internal: Generate percussive slam
local function generateRewardSlam(duration, volume)
    local sampleRate = 44100
    local samples = math.floor(sampleRate * duration)
    local soundData = love.sound.newSoundData(samples, sampleRate, 16, 1)

    volume = volume or 0.4

    for i = 0, samples - 1 do
        local progress = i / samples
        local envelope = math.exp(-progress * 20)
        local crack = 0
        if progress < 0.05 then
            crack = (math.random() * 2 - 1) * (1 - progress / 0.05)
        end
        local body = (math.random() * 2 - 1) * 0.6
        local mid = (math.random() * 2 - 1) * math.exp(-progress * 30) * 0.4
        local low = (math.random() * 2 - 1) * math.exp(-progress * 10) * 0.3
        local sample = (crack * 0.7 + body * 0.3 + mid + low) * volume * envelope
        soundData:setSample(i, math.max(-1, math.min(1, sample)))
    end

    return love.audio.newSource(soundData)
end

-- Internal: Generate atonal thunk
local function generateAtonalThunk(duration, volume)
    local sampleRate = 44100
    local samples = math.floor(sampleRate * duration)
    local soundData = love.sound.newSoundData(samples, sampleRate, 16, 1)

    volume = volume or 0.35

    for i = 0, samples - 1 do
        local progress = i / samples
        local envelope = 0
        if progress < 0.02 then
            envelope = progress / 0.02
        else
            envelope = math.exp(-(progress - 0.02) * 18)
        end
        local high = (math.random() * 2 - 1) * math.exp(-progress * 50) * 0.4
        local mid = (math.random() * 2 - 1) * math.exp(-progress * 20) * 0.5
        local low = (math.random() * 2 - 1) * math.exp(-progress * 8) * 0.4
        local sample = (high + mid + low) * volume * envelope
        soundData:setSample(i, math.max(-1, math.min(1, sample)))
    end

    return love.audio.newSource(soundData)
end

-- Internal: Generate click
local function generateClick(duration, volume)
    local sampleRate = 44100
    local samples = math.floor(sampleRate * duration)
    local soundData = love.sound.newSoundData(samples, sampleRate, 16, 1)

    volume = volume or 0.25

    for i = 0, samples - 1 do
        local t = i / sampleRate
        local progress = i / samples
        local envelope = math.exp(-progress * 40)
        local click = 0
        if progress < 0.1 then
            click = (math.random() * 2 - 1) * (1 - progress * 10)
        end
        local body = math.sin(2 * math.pi * 2000 * t) * 0.3 * math.exp(-progress * 60)
        local sample = (click + body) * volume * envelope
        soundData:setSample(i, math.max(-1, math.min(1, sample)))
    end

    return love.audio.newSource(soundData)
end

-- Internal: Generate drumroll
local function generateDrumroll(duration, volume)
    local sampleRate = 44100
    local samples = math.floor(sampleRate * duration)
    local soundData = love.sound.newSoundData(samples, sampleRate, 16, 1)

    volume = volume or 0.3
    local hitRate = 30

    for i = 0, samples - 1 do
        local t = i / sampleRate
        local progress = i / samples
        local hitPhase = (t * hitRate) % 1
        local hitEnvelope = math.exp(-hitPhase * 8)
        local variation = math.sin(t * 127) * 0.3 + math.sin(t * 83) * 0.2
        local noise = (math.random() * 2 - 1)
        local overallEnv = 1 - progress * 0.3
        local sample = noise * hitEnvelope * (0.6 + variation * 0.4) * volume * overallEnv
        soundData:setSample(i, math.max(-1, math.min(1, sample)))
    end

    local source = love.audio.newSource(soundData)
    source:setLooping(true)
    return source
end

-- Internal: Generate whir with thunk ending
local function generateWhir(duration, volume)
    local sampleRate = 44100
    local thunkDuration = 0.1
    local totalDuration = duration + thunkDuration
    local samples = math.floor(sampleRate * totalDuration)
    local soundData = love.sound.newSoundData(samples, sampleRate, 16, 1)

    volume = volume or 0.3
    local filtered = 0
    local filterStrength = 0.85
    local whirSamples = math.floor(sampleRate * duration)

    for i = 0, samples - 1 do
        local t = i / sampleRate
        local sample = 0

        if i < whirSamples then
            local progress = i / whirSamples
            local envelope = math.sin(progress * math.pi)
            envelope = envelope * envelope
            local noise = math.random() * 2 - 1
            filtered = filtered * filterStrength + noise * (1 - filterStrength)
            local modSpeed = 25 + progress * 15
            local modulation = 0.7 + math.sin(2 * math.pi * modSpeed * t) * 0.3
            sample = filtered * modulation * volume * envelope
        else
            local thunkProgress = (i - whirSamples) / (samples - whirSamples)
            local thunkEnv = math.exp(-thunkProgress * 20)
            local crack = (math.random() * 2 - 1) * math.exp(-thunkProgress * 60) * 0.5
            local body = (math.random() * 2 - 1) * math.exp(-thunkProgress * 20) * 0.6
            local tail = (math.random() * 2 - 1) * math.exp(-thunkProgress * 8) * 0.3
            sample = (crack + body + tail) * volume * 1.3 * thunkEnv
        end

        soundData:setSample(i, math.max(-1, math.min(1, sample)))
    end

    return love.audio.newSource(soundData)
end

-- Internal: Generate tonal thunk
local function generateThunk(frequency, duration, volume)
    local sampleRate = 44100
    local samples = math.floor(sampleRate * duration)
    local soundData = love.sound.newSoundData(samples, sampleRate, 16, 1)

    volume = volume or 0.3

    for i = 0, samples - 1 do
        local t = i / sampleRate
        local progress = i / samples
        local envelope = 0
        if progress < 0.02 then
            envelope = progress / 0.02
        elseif progress < 0.1 then
            envelope = 1 - (progress - 0.02) / 0.08 * 0.5
        else
            envelope = 0.5 * math.exp(-(progress - 0.1) * 8)
        end
        local lowFreq = frequency
        local midFreq = frequency * 2.5
        local thump = math.sin(2 * math.pi * lowFreq * t) * 0.7
        local punch = math.sin(2 * math.pi * midFreq * t) * 0.3 * math.exp(-progress * 15)
        local noise = (math.random() * 2 - 1) * 0.15 * math.exp(-progress * 20)
        local sample = (thump + punch + noise) * volume * envelope
        soundData:setSample(i, math.max(-1, math.min(1, sample)))
    end

    return love.audio.newSource(soundData)
end

--------------------------------------------------
-- PUBLIC API
--------------------------------------------------

--- Initialize the audio system with default sounds
function sfx.init()
    if initialized then return end

    -- Success chime - bright major chord ascending
    sounds.success = sfx.generateChord({523.25, 659.25, 783.99}, 0.4, 0.25, "sine")

    -- Partial/ambiguous - uncertain tone
    sounds.partial = sfx.generateChord({349.23, 440.00}, 0.5, 0.2, "triangle")

    -- Reject - low muted tone
    sounds.reject = sfx.generateTone(220, 0.3, 0.2, "triangle")

    -- Stabilize - calming tone
    sounds.stabilize = sfx.generateChord({261.63, 329.63, 392.00}, 0.6, 0.2, "sine")

    -- Destabilize - dissonant
    sounds.destabilize = sfx.generateChord({233.08, 246.94}, 0.4, 0.15, "triangle")

    -- UI blip - short click
    sounds.blip = sfx.generateTone(880, 0.08, 0.15, "sine")

    -- UI navigation - soft tick
    sounds.ui_navigate = sfx.generateTone(660, 0.04, 0.12, "sine")

    -- UI select/confirm - pleasant two-note
    sounds.ui_select = sfx.generateChord({523.25, 659.25}, 0.12, 0.18, "sine")

    -- UI back/cancel - softer descending
    sounds.ui_back = sfx.generateTone(392, 0.1, 0.12, "triangle")

    -- UI slider adjust - very subtle tick
    sounds.ui_adjust = sfx.generateTone(550, 0.03, 0.08, "sine")

    -- Coin collect
    sounds.coin = sfx.generateChord({987.77, 1318.51}, 0.15, 0.2, "sine")

    -- Thunk - atonal percussive impact for UI slams
    sounds.thunk = generateAtonalThunk(0.15, 0.35)

    -- Click - short percussive click for selection
    sounds.click = generateClick(0.03, 0.25)

    -- Reward pop - satisfying snap for small rewards
    sounds.reward_pop = generateRewardPop(0.1, 0.3)

    -- Reward slam - bigger impact for milestones
    sounds.reward_slam = generateRewardSlam(0.2, 0.4)

    -- Drumroll - continuous roll for score roll-up
    sounds.drumroll = generateDrumroll(0.5, 0.25)

    -- Whir - short machine whirring sound for processing
    sounds.whir = generateWhir(0.25, 0.3)

    -- Ambient drone (low, subtle)
    sounds.drone = sfx.generateChord({65.41, 98.00}, 2.0, 0.08, "sine")

    -- End/fade tone
    sounds.ending = sfx.generateChord({261.63, 392.00, 523.25}, 1.5, 0.15, "sine")

    -- Footstep variations (soft, subtle)
    sounds.footstep1 = sfx.generateFootstep(60, 0.12, 0.15)
    sounds.footstep2 = sfx.generateFootstep(55, 0.11, 0.14)
    sounds.footstep3 = sfx.generateFootstep(65, 0.13, 0.13)
    sounds.footstep4 = sfx.generateFootstep(58, 0.12, 0.15)

    initialized = true
end

--- Play a sound by name
-- @param name Sound name
-- @param volume Optional volume multiplier (default 1)
-- @param pitch Optional pitch multiplier (default 1)
function sfx.play(name, volume, pitch)
    if not initialized then sfx.init() end

    local sound = sounds[name]
    if sound then
        sound:stop()
        local finalVolume = (volume or 1) * sfxVolume
        sound:setVolume(finalVolume)
        sound:setPitch(pitch or 1.0)
        sound:play()
    end
end

--- Set master SFX volume
-- @param vol Volume (0-1)
function sfx.setSfxVolume(vol)
    sfxVolume = math.max(0, math.min(1, vol))
end

--- Get master SFX volume
-- @return number
function sfx.getSfxVolume()
    return sfxVolume
end

--- Check if a sound exists
-- @param name Sound name
-- @return boolean
function sfx.has(name)
    return sounds[name] ~= nil
end

--- Stop a sound
-- @param name Sound name
function sfx.stop(name)
    if sounds[name] then
        sounds[name]:stop()
    end
end

--- Stop all sounds
function sfx.stopAll()
    for _, sound in pairs(sounds) do
        sound:stop()
    end
end

--- Get list of available sounds
-- @return table
function sfx.list()
    local list = {}
    for name, _ in pairs(sounds) do
        table.insert(list, name)
    end
    return list
end

--- Add a custom sound
-- @param name Sound name
-- @param source LOVE audio source
function sfx.addSound(name, source)
    sounds[name] = source
end

--- Remove a sound
-- @param name Sound name
function sfx.removeSound(name)
    if sounds[name] then
        sounds[name]:stop()
        sounds[name] = nil
    end
end

--- Update footstep sounds based on player movement
-- @param dt Delta time
-- @param isMoving Whether the player is moving
-- @param speedFactor Optional speed factor (0-1, affects interval)
function sfx.updateFootsteps(dt, isMoving, speedFactor)
    if not initialized then sfx.init() end

    speedFactor = speedFactor or 1
    speedFactor = math.max(0.2, speedFactor)
    local currentInterval = footsteps.interval / speedFactor

    if isMoving then
        footsteps.timer = footsteps.timer + dt
        if footsteps.timer >= currentInterval then
            footsteps.timer = 0
            footsteps.lastStep = (footsteps.lastStep % 4) + 1
            local stepSound = sounds["footstep" .. footsteps.lastStep]
            if stepSound then
                stepSound:stop()
                stepSound:setVolume(footsteps.volume * sfxVolume)
                stepSound:play()
            end
        end
    else
        footsteps.timer = currentInterval * 0.5
    end
end

--- Set footstep parameters
-- @param interval Time between footsteps (default 0.28)
-- @param volume Footstep volume (default 0.28)
function sfx.setFootstepParams(interval, volume)
    if interval then footsteps.interval = interval end
    if volume then footsteps.volume = volume end
end

--- Get footstep parameters
-- @return interval, volume
function sfx.getFootstepParams()
    return footsteps.interval, footsteps.volume
end

return sfx
