--[[
    love2d-settings
    Simple persistent settings system for LOVE 2D

    Provides key-value storage with automatic file persistence using
    LOVE's save directory. Perfect for game options, preferences, and state.

    USAGE:
        local settings = require("libs.settings")

        function love.load()
            -- Define your settings schema with defaults
            settings.init({
                musicVolume = 0.5,
                sfxVolume = 0.8,
                fullscreen = false,
                difficulty = "normal",
                highScore = 0
            })

            -- Load saved settings (uses defaults for missing keys)
            settings.load()

            -- Apply loaded settings
            love.audio.setVolume(settings.get("musicVolume"))
        end

        function onVolumeChange(newVolume)
            settings.set("musicVolume", newVolume)
            settings.save()  -- Persist to disk
        end

    API:
        settings.init(defaults, filename)
            Initialize with default values and optional filename
            defaults: Table of key-value pairs with default values
            filename: Optional save filename (default "settings.dat")

        settings.load()
            Load settings from file, using defaults for missing keys

        settings.save()
            Save current settings to file

        settings.get(key) -> value
            Get a setting value

        settings.set(key, value)
            Set a setting value (only for keys in defaults)

        settings.getAll() -> table
            Get all settings as a table copy

        settings.resetToDefaults()
            Reset all settings to default values

        settings.has(key) -> boolean
            Check if a setting key exists

        settings.getDefault(key) -> value
            Get the default value for a key

        settings.onChange(callback)
            Register a callback for setting changes
            callback(key, newValue, oldValue)

    LICENSE: MIT
]]

local settings = {}

-- Internal state
local defaults = {}
local current = {}
local filename = "settings.dat"
local changeCallbacks = {}

--------------------------------------------------
-- PUBLIC API
--------------------------------------------------

--- Initialize the settings system
-- @param defaultValues Table of default key-value pairs
-- @param saveFilename Optional filename (default "settings.dat")
function settings.init(defaultValues, saveFilename)
    defaults = defaultValues or {}
    filename = saveFilename or "settings.dat"

    -- Initialize current values from defaults
    current = {}
    for k, v in pairs(defaults) do
        current[k] = v
    end
end

--- Load settings from file
function settings.load()
    local info = love.filesystem.getInfo(filename)
    if info then
        local contents = love.filesystem.read(filename)
        if contents then
            -- Parse simple key=value format
            for line in contents:gmatch("[^\r\n]+") do
                local key, value = line:match("^([%w_]+)=(.+)$")
                if key and value and defaults[key] ~= nil then
                    -- Type coercion based on default type
                    local defaultType = type(defaults[key])
                    if defaultType == "number" then
                        current[key] = tonumber(value)
                    elseif defaultType == "boolean" then
                        current[key] = value == "true"
                    else
                        current[key] = value
                    end
                end
            end
        end
    end
end

--- Save settings to file
function settings.save()
    local lines = {}
    for k, v in pairs(current) do
        table.insert(lines, k .. "=" .. tostring(v))
    end
    love.filesystem.write(filename, table.concat(lines, "\n"))
end

--- Get a setting value
-- @param key Setting key
-- @return value or nil
function settings.get(key)
    return current[key]
end

--- Set a setting value
-- @param key Setting key (must exist in defaults)
-- @param value New value
-- @return boolean success
function settings.set(key, value)
    if defaults[key] == nil then
        return false
    end

    local oldValue = current[key]
    current[key] = value

    -- Notify listeners
    for _, callback in ipairs(changeCallbacks) do
        callback(key, value, oldValue)
    end

    return true
end

--- Get all settings as a table copy
-- @return table
function settings.getAll()
    local copy = {}
    for k, v in pairs(current) do
        copy[k] = v
    end
    return copy
end

--- Reset all settings to defaults
function settings.resetToDefaults()
    for k, v in pairs(defaults) do
        local oldValue = current[k]
        current[k] = v

        -- Notify listeners
        if oldValue ~= v then
            for _, callback in ipairs(changeCallbacks) do
                callback(k, v, oldValue)
            end
        end
    end
end

--- Check if a setting key exists
-- @param key Setting key
-- @return boolean
function settings.has(key)
    return defaults[key] ~= nil
end

--- Get the default value for a key
-- @param key Setting key
-- @return value or nil
function settings.getDefault(key)
    return defaults[key]
end

--- Get all default values
-- @return table
function settings.getDefaults()
    local copy = {}
    for k, v in pairs(defaults) do
        copy[k] = v
    end
    return copy
end

--- Register a callback for setting changes
-- @param callback function(key, newValue, oldValue)
-- @return id for unregistering
function settings.onChange(callback)
    table.insert(changeCallbacks, callback)
    return #changeCallbacks
end

--- Unregister a change callback
-- @param id Callback ID from onChange
function settings.offChange(id)
    changeCallbacks[id] = nil
end

--- Clear all change callbacks
function settings.clearCallbacks()
    changeCallbacks = {}
end

--------------------------------------------------
-- CONVENIENCE METHODS
--------------------------------------------------

--- Increment a numeric setting
-- @param key Setting key
-- @param amount Amount to add (default 1)
-- @param min Optional minimum value
-- @param max Optional maximum value
-- @return new value or nil
function settings.increment(key, amount, min, max)
    local value = current[key]
    if type(value) ~= "number" then return nil end

    amount = amount or 1
    value = value + amount

    if min then value = math.max(min, value) end
    if max then value = math.min(max, value) end

    settings.set(key, value)
    return value
end

--- Toggle a boolean setting
-- @param key Setting key
-- @return new value or nil
function settings.toggle(key)
    local value = current[key]
    if type(value) ~= "boolean" then return nil end

    settings.set(key, not value)
    return not value
end

--- Cycle through a list of values
-- @param key Setting key
-- @param values Table of possible values
-- @return new value or nil
function settings.cycle(key, values)
    local current_value = current[key]

    for i, v in ipairs(values) do
        if v == current_value then
            local next_index = (i % #values) + 1
            settings.set(key, values[next_index])
            return values[next_index]
        end
    end

    -- Current value not in list, use first
    settings.set(key, values[1])
    return values[1]
end

return settings
