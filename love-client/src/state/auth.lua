-- Authentication state management
local settings = require("libs.LoveLibs.core.settings")

local auth = {}

-- Initialize auth settings for token persistence
function auth.init()
    settings.init({
        token = "",
        playerId = "",
    }, "auth.dat")
    settings.load()
end

-- Get stored token
function auth.getToken()
    local token = settings.get("token")
    if token and token ~= "" then
        return token
    end
    return nil
end

-- Get stored player ID
function auth.getPlayerId()
    local playerId = settings.get("playerId")
    if playerId and playerId ~= "" then
        return playerId
    end
    return nil
end

-- Store authentication
function auth.setAuth(token, playerId)
    settings.set("token", token or "")
    settings.set("playerId", playerId or "")
    settings.save()
end

-- Clear authentication
function auth.clear()
    settings.set("token", "")
    settings.set("playerId", "")
    settings.save()
end

-- Check if authenticated
function auth.isAuthenticated()
    return auth.getToken() ~= nil
end

return auth
