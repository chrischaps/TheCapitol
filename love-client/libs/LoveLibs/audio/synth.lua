--[[
    Procedural Audio Synthesizer for LÖVE
    =====================================
    Generate sound effects programmatically without audio files.
    Perfect for retro games, prototypes, or dynamic audio.

    Usage:
        local Synth = require('LoveLibs.audio.synth')

        -- Create a synthesizer
        local synth = Synth.new()

        -- Generate built-in sounds
        local jump = synth:create_sound('jump')
        local hit = synth:create_sound('hit')
        local pickup = synth:create_sound('pickup')

        -- Play sounds
        jump:play()
        hit:play()

        -- Or create custom sounds
        local custom = synth:generate({
            duration = 0.2,
            frequency = {start = 440, finish = 880},  -- Frequency sweep
            waveform = 'sine',
            envelope = {attack = 0.01, decay = 0.1, sustain = 0.5, release = 0.09},
            volume = 0.5
        })

    Requires: LÖVE framework (love.audio, love.sound)
]]

local Synth = {}
Synth.__index = Synth

-- Audio settings
local SAMPLE_RATE = 44100
local BIT_DEPTH = 16

--------------------------------------------------------------------------------
-- Waveform Generators (return -1 to 1)
--------------------------------------------------------------------------------

local Waveforms = {}

function Waveforms.sine(phase)
    return math.sin(phase * 2 * math.pi)
end

function Waveforms.saw(phase)
    return 2 * (phase % 1) - 1
end

function Waveforms.square(phase, duty)
    duty = duty or 0.5
    return (phase % 1) < duty and 1 or -1
end

function Waveforms.triangle(phase)
    local p = phase % 1
    return 4 * math.abs(p - 0.5) - 1
end

function Waveforms.noise()
    return math.random() * 2 - 1
end

-- Pulse wave with variable duty cycle
function Waveforms.pulse(phase, duty)
    duty = duty or 0.25
    return (phase % 1) < duty and 1 or -1
end

--------------------------------------------------------------------------------
-- Envelope Functions
--------------------------------------------------------------------------------

local Envelopes = {}

-- ADSR envelope (Attack, Decay, Sustain level, Release)
function Envelopes.adsr(t, attack, decay, sustain, release, duration)
    if t < attack then
        return t / attack
    elseif t < attack + decay then
        return 1 - (1 - sustain) * ((t - attack) / decay)
    elseif t < duration - release then
        return sustain
    elseif t < duration then
        return sustain * (1 - (t - (duration - release)) / release)
    else
        return 0
    end
end

-- Exponential decay
function Envelopes.exp_decay(t, decay_rate)
    return math.exp(-t * decay_rate)
end

-- Punch envelope (quick attack, exponential decay)
function Envelopes.punch(t, duration)
    local attack = 0.01
    if t < attack then
        return t / attack
    else
        return math.exp(-(t - attack) * 8) * (1 - (t / duration) * 0.5)
    end
end

-- Linear fade out
function Envelopes.fade_out(t, duration)
    return 1 - (t / duration)
end

-- Linear fade in then out
function Envelopes.fade_in_out(t, duration)
    local mid = duration / 2
    if t < mid then
        return t / mid
    else
        return 1 - (t - mid) / mid
    end
end

--------------------------------------------------------------------------------
-- Sound Generator
--------------------------------------------------------------------------------

function Synth.new()
    local self = setmetatable({}, Synth)
    self.sounds = {}
    return self
end

-- Generate a sound from parameters
function Synth:generate(params)
    params = params or {}

    local duration = params.duration or 0.2
    local samples = math.floor(SAMPLE_RATE * duration)
    local data = love.sound.newSoundData(samples, SAMPLE_RATE, BIT_DEPTH, 1)

    -- Frequency (can be number or table with start/finish for sweep)
    local freq_start = type(params.frequency) == "table" and params.frequency.start or params.frequency or 440
    local freq_end = type(params.frequency) == "table" and params.frequency.finish or freq_start

    -- Waveform
    local waveform = params.waveform or 'sine'
    local wave_func = Waveforms[waveform] or Waveforms.sine

    -- Envelope
    local env = params.envelope or {}
    local attack = env.attack or 0.01
    local decay = env.decay or 0.05
    local sustain = env.sustain or 0.7
    local release = env.release or 0.1

    -- Volume
    local volume = params.volume or 0.5

    -- Optional noise mix
    local noise_amount = params.noise or 0

    -- Optional vibrato
    local vibrato_speed = params.vibrato_speed or 0
    local vibrato_depth = params.vibrato_depth or 0

    -- Phase accumulator for smooth frequency changes
    local phase = 0

    for i = 0, samples - 1 do
        local t = i / SAMPLE_RATE
        local progress = t / duration

        -- Frequency interpolation (linear sweep)
        local freq = freq_start + (freq_end - freq_start) * progress

        -- Apply vibrato
        if vibrato_speed > 0 then
            freq = freq + math.sin(t * vibrato_speed * 2 * math.pi) * vibrato_depth
        end

        -- Accumulate phase
        phase = phase + freq / SAMPLE_RATE

        -- Generate waveform
        local sample = wave_func(phase)

        -- Mix in noise
        if noise_amount > 0 then
            sample = sample * (1 - noise_amount) + Waveforms.noise() * noise_amount
        end

        -- Apply envelope
        local env_value = Envelopes.adsr(t, attack, decay, sustain, release, duration)
        sample = sample * env_value * volume

        -- Clamp to valid range
        sample = math.max(-1, math.min(1, sample))

        data:setSample(i, sample)
    end

    return love.audio.newSource(data, "static")
end

-- Create a looping sound
function Synth:generate_loop(params)
    local source = self:generate(params)
    source:setLooping(true)
    return source
end

--------------------------------------------------------------------------------
-- Preset Sounds
--------------------------------------------------------------------------------

local Presets = {}

Presets.jump = function(self)
    return self:generate({
        duration = 0.15,
        frequency = {start = 280, finish = 580},
        waveform = 'sine',
        envelope = {attack = 0.008, decay = 0.02, sustain = 0.7, release = 0.05},
        volume = 0.4,
        noise = 0.05
    })
end

Presets.land = function(self)
    return self:generate({
        duration = 0.18,
        frequency = {start = 120, finish = 60},
        waveform = 'sine',
        envelope = {attack = 0.005, decay = 0.15, sustain = 0.2, release = 0.02},
        volume = 0.5,
        noise = 0.15
    })
end

Presets.hit = function(self)
    return self:generate({
        duration = 0.1,
        frequency = {start = 200, finish = 80},
        waveform = 'square',
        envelope = {attack = 0.001, decay = 0.08, sustain = 0.1, release = 0.01},
        volume = 0.4,
        noise = 0.3
    })
end

Presets.pickup = function(self)
    return self:generate({
        duration = 0.2,
        frequency = {start = 500, finish = 1200},
        waveform = 'triangle',
        envelope = {attack = 0.01, decay = 0.05, sustain = 0.6, release = 0.09},
        volume = 0.35
    })
end

Presets.coin = function(self)
    return self:generate({
        duration = 0.15,
        frequency = {start = 800, finish = 1000},
        waveform = 'square',
        envelope = {attack = 0.001, decay = 0.05, sustain = 0.5, release = 0.05},
        volume = 0.25
    })
end

Presets.laser = function(self)
    return self:generate({
        duration = 0.12,
        frequency = {start = 1500, finish = 200},
        waveform = 'saw',
        envelope = {attack = 0.001, decay = 0.1, sustain = 0.3, release = 0.01},
        volume = 0.3
    })
end

Presets.explosion = function(self)
    return self:generate({
        duration = 0.5,
        frequency = {start = 100, finish = 30},
        waveform = 'noise',
        envelope = {attack = 0.001, decay = 0.4, sustain = 0.1, release = 0.09},
        volume = 0.6
    })
end

Presets.powerup = function(self)
    return self:generate({
        duration = 0.4,
        frequency = {start = 300, finish = 900},
        waveform = 'triangle',
        envelope = {attack = 0.02, decay = 0.1, sustain = 0.7, release = 0.18},
        volume = 0.4,
        vibrato_speed = 8,
        vibrato_depth = 20
    })
end

Presets.hurt = function(self)
    return self:generate({
        duration = 0.25,
        frequency = {start = 400, finish = 100},
        waveform = 'square',
        envelope = {attack = 0.001, decay = 0.15, sustain = 0.3, release = 0.09},
        volume = 0.4,
        noise = 0.2
    })
end

Presets.blip = function(self)
    return self:generate({
        duration = 0.05,
        frequency = 600,
        waveform = 'square',
        envelope = {attack = 0.001, decay = 0.04, sustain = 0.5, release = 0.005},
        volume = 0.3
    })
end

-- Create a preset sound by name
function Synth:create_sound(name)
    if Presets[name] then
        return Presets[name](self)
    else
        error("Unknown sound preset: " .. tostring(name))
    end
end

-- List available presets
function Synth:list_presets()
    local names = {}
    for name, _ in pairs(Presets) do
        table.insert(names, name)
    end
    table.sort(names)
    return names
end

--------------------------------------------------------------------------------
-- Utility: Play with variation
--------------------------------------------------------------------------------

-- Play a sound with slight pitch variation for organic feel
function Synth.play_varied(source, pitch_variation, volume_variation)
    pitch_variation = pitch_variation or 0.05
    volume_variation = volume_variation or 0.1

    local clone = source:clone()
    clone:setPitch(1 + (math.random() * 2 - 1) * pitch_variation)
    clone:setVolume(source:getVolume() * (1 + (math.random() * 2 - 1) * volume_variation))
    clone:play()
    return clone
end

--------------------------------------------------------------------------------
-- Module exports
--------------------------------------------------------------------------------

Synth.Waveforms = Waveforms
Synth.Envelopes = Envelopes
Synth.Presets = Presets

return Synth
