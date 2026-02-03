-- REST API client using luasocket
local socket = require("socket")
local http = require("socket.http")
local ltn12 = require("ltn12")
local json = require("libs.json")
local config = require("src.config")

local api = {}

-- Parse URL into host, port, path
local function parseUrl(url)
    local protocol, host, port, path = url:match("^(https?)://([^:/]+):?(%d*)(.*)$")
    port = port ~= "" and tonumber(port) or (protocol == "https" and 443 or 80)
    path = path ~= "" and path or "/"
    return host, port, path
end

-- Make HTTP request
local function request(method, endpoint, body, token)
    local url = config.API_URL .. endpoint
    local response_body = {}

    local headers = {
        ["Content-Type"] = "application/json",
        ["Accept"] = "application/json",
    }

    if token then
        headers["Authorization"] = "Bearer " .. token
    end

    local request_body = nil
    if body then
        request_body = json.encode(body)
        headers["Content-Length"] = tostring(#request_body)
    end

    -- Use pcall to catch any connection errors
    local success, result_or_err, status_code, response_headers = pcall(function()
        return http.request({
            url = url,
            method = method,
            headers = headers,
            source = request_body and ltn12.source.string(request_body) or nil,
            sink = ltn12.sink.table(response_body),
        })
    end)

    if not success then
        -- pcall failed - lua error
        return {
            ok = false,
            status = 0,
            data = { error = "Request failed: " .. tostring(result_or_err) },
            raw = "",
        }
    end

    -- result_or_err is now the first return value (1 for success, nil for failure)
    -- status_code is the HTTP status or error message
    local result = result_or_err

    -- Check if request itself failed (result is nil, status_code contains error)
    if result == nil then
        return {
            ok = false,
            status = 0,
            data = { error = "Connection failed: " .. tostring(status_code) },
            raw = "",
        }
    end

    local response_str = table.concat(response_body)
    local response_data = nil

    if response_str and #response_str > 0 then
        local ok, parsed = pcall(json.decode, response_str)
        if ok then
            response_data = parsed
        end
    end

    -- Ensure status_code is a number
    local status = tonumber(status_code) or 0

    return {
        ok = status >= 200 and status < 300,
        status = status,
        data = response_data,
        raw = response_str,
    }
end

-- Authentication

function api.login(email, password)
    return request("POST", "/auth/login", {
        email = email,
        password = password,
    })
end

function api.register(email, password, name)
    return request("POST", "/auth/register", {
        email = email,
        password = password,
        player_name = name,  -- backend expects player_name, not name
    })
end

-- World

function api.getNearby(token)
    return request("GET", "/world/nearby", nil, token)
end

-- Player

function api.getInventory(token)
    return request("GET", "/player/inventory", nil, token)
end

return api
