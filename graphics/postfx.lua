--[[
    PostFX - Post-Processing Effects Library for LÖVE2D

    A configurable multi-pass shader pipeline with:
    - Water/wave distortion
    - Bloom/glow effect
    - Color grading with vignette
    - Dithering to reduce banding

    Usage:
        local PostFX = require("LoveLibs.graphics.postfx")

        function love.load()
            PostFX.init(1280, 720, {
                distortion = { enabled = true, strength = 0.006 },
                bloom = { enabled = true, threshold = 0.5, intensity = 0.6 },
                colorGrade = { enabled = true, warmth = 0.1, saturation = 0.1 }
            })
        end

        function love.update(dt)
            PostFX.update(dt)
        end

        function love.draw()
            PostFX.beginScene()
            -- Draw your game here
            PostFX.endScene(scale, offsetX, offsetY)
        end

        function love.resize(w, h)
            PostFX.resize(nativeW, nativeH)
        end
]]

local PostFX = {}

-- Internal state
local startTime = 0
local elapsedTime = 0

-- Canvases
local mainCanvas = nil
local distortionCanvas = nil
local brightnessCanvas = nil
local blurTempCanvas = nil
local bloomCanvas = nil
local compositeCanvas = nil

-- Shaders
local distortionShader = nil
local brightnessShader = nil
local blurHorizontalShader = nil
local blurVerticalShader = nil
local colorGradeShader = nil

-- Public state
PostFX.bypass = false
PostFX.debugLevel = 0

-- Default configuration
PostFX.config = {
    distortion = {
        enabled = true,
        strength = 0.006,
        speed = 0.7
    },
    bloom = {
        enabled = true,
        threshold = 0.50,
        softness = 0.15,
        intensity = 0.6,
        radius = 3.0
    },
    colorGrade = {
        enabled = true,
        saturation = 0.10,
        contrast = 0.08,
        warmth = 0.04,
        vignetteStrength = 0.30,
        vignetteRadius = 0.70,
        -- Shadow/highlight tinting (RGB multipliers)
        shadowTint = {0.08, 0.18, 0.22},    -- Added to shadows
        highlightTint = {1.0, 0.98, 0.95},  -- Multiplied with highlights
    }
}

--============================================================================
-- SHADER SOURCE CODE
--============================================================================

local distortionSource = [[
    extern number time;
    extern number strength;
    extern number speed;

    vec4 effect(vec4 vcolor, Image tex, vec2 tc, vec2 sc) {
        number wave1 = sin(tc.y * 15.0 + time * speed) * strength;
        number wave2 = sin(tc.y * 23.0 - time * speed * 0.7) * strength * 0.5;
        number wave3 = sin(tc.x * 12.0 + time * speed * 0.5) * strength * 0.3;

        vec2 offset = vec2(wave1 + wave2, wave3 * 0.4);
        offset = offset * (0.5 + tc.y * 0.5);

        vec2 uv = tc + offset;
        uv = clamp(uv, vec2(0.001), vec2(0.999));

        return Texel(tex, uv) * vcolor;
    }
]]

local brightnessSource = [[
    extern float threshold;
    extern float softness;

    vec4 effect(vec4 color, Image tex, vec2 uv, vec2 screen_coords) {
        vec4 texcolor = Texel(tex, uv);
        float luma = dot(texcolor.rgb, vec3(0.299, 0.587, 0.114));
        float brightness = smoothstep(threshold - softness, threshold + softness, luma);
        float maxChannel = max(texcolor.r, max(texcolor.g, texcolor.b));
        float satBoost = smoothstep(0.6, 0.9, maxChannel) * 0.3;
        return vec4(texcolor.rgb * (brightness + satBoost), texcolor.a);
    }
]]

local blurHorizontalSource = [[
    extern float blur_size;

    vec4 effect(vec4 color, Image tex, vec2 uv, vec2 screen_coords) {
        vec4 result = vec4(0.0);
        result += Texel(tex, uv + vec2(-4.0 * blur_size, 0.0)) * 0.028532;
        result += Texel(tex, uv + vec2(-3.0 * blur_size, 0.0)) * 0.067234;
        result += Texel(tex, uv + vec2(-2.0 * blur_size, 0.0)) * 0.124009;
        result += Texel(tex, uv + vec2(-1.0 * blur_size, 0.0)) * 0.179044;
        result += Texel(tex, uv + vec2( 0.0 * blur_size, 0.0)) * 0.20236;
        result += Texel(tex, uv + vec2( 1.0 * blur_size, 0.0)) * 0.179044;
        result += Texel(tex, uv + vec2( 2.0 * blur_size, 0.0)) * 0.124009;
        result += Texel(tex, uv + vec2( 3.0 * blur_size, 0.0)) * 0.067234;
        result += Texel(tex, uv + vec2( 4.0 * blur_size, 0.0)) * 0.028532;
        return result * color;
    }
]]

local blurVerticalSource = [[
    extern float blur_size;

    vec4 effect(vec4 color, Image tex, vec2 uv, vec2 screen_coords) {
        vec4 result = vec4(0.0);
        result += Texel(tex, uv + vec2(0.0, -4.0 * blur_size)) * 0.028532;
        result += Texel(tex, uv + vec2(0.0, -3.0 * blur_size)) * 0.067234;
        result += Texel(tex, uv + vec2(0.0, -2.0 * blur_size)) * 0.124009;
        result += Texel(tex, uv + vec2(0.0, -1.0 * blur_size)) * 0.179044;
        result += Texel(tex, uv + vec2(0.0,  0.0 * blur_size)) * 0.20236;
        result += Texel(tex, uv + vec2(0.0,  1.0 * blur_size)) * 0.179044;
        result += Texel(tex, uv + vec2(0.0,  2.0 * blur_size)) * 0.124009;
        result += Texel(tex, uv + vec2(0.0,  3.0 * blur_size)) * 0.067234;
        result += Texel(tex, uv + vec2(0.0,  4.0 * blur_size)) * 0.028532;
        return result * color;
    }
]]

local colorGradeSource = [[
    extern float saturation;
    extern float contrast;
    extern float warmth;
    extern float vignetteStrength;
    extern float vignetteRadius;
    extern float time;
    extern vec3 shadowTint;
    extern vec3 highlightTint;

    float dither(vec2 pos) {
        return fract(52.9829189 * fract(dot(pos, vec2(0.06711056, 0.00583715))));
    }

    vec4 effect(vec4 color, Image tex, vec2 uv, vec2 screen_coords) {
        vec4 texcolor = Texel(tex, uv);
        vec3 result = texcolor.rgb;

        // Contrast
        result = mix(vec3(0.5), result, 1.0 + contrast);

        // Saturation
        float gray = dot(result, vec3(0.299, 0.587, 0.114));
        result = mix(vec3(gray), result, 1.0 + saturation);

        // Warmth (orange-blue shift)
        result.r = result.r + warmth * 0.4;
        result.g = result.g + warmth * 0.1;
        result.b = result.b - warmth * 0.3;

        // Vignette
        vec2 center = uv - vec2(0.5);
        float dist = length(center);
        float vig = 1.0 - smoothstep(vignetteRadius - 0.25, vignetteRadius + 0.35, dist) * vignetteStrength;
        result = result * vig;

        // Shadow/highlight tinting
        float luma = dot(result, vec3(0.299, 0.587, 0.114));
        result.r = result.r + shadowTint.r * (1.0 - luma) * 0.15 + luma * (highlightTint.r - 1.0) * 0.5;
        result.g = result.g + shadowTint.g * (1.0 - luma) * 0.15 + luma * (highlightTint.g - 1.0) * 0.5;
        result.b = result.b + shadowTint.b * (1.0 - luma) * 0.15 + luma * (highlightTint.b - 1.0) * 0.5;

        // Dithering
        float ditherAmount = 0.5 / 255.0;
        float noise = dither(screen_coords + vec2(time * 100.0, 0.0));
        result = result + vec3(ditherAmount) * (noise - 0.5) * 2.0;

        result = clamp(result, vec3(0.0), vec3(1.0));
        return vec4(result, texcolor.a);
    }
]]

--============================================================================
-- PUBLIC API
--============================================================================

--- Initialize the post-processing pipeline
--- @param w number Native width (content resolution)
--- @param h number Native height (content resolution)
--- @param config table|nil Optional configuration overrides
function PostFX.init(w, h, config)
    startTime = love.timer.getTime()

    w = w or love.graphics.getWidth()
    h = h or love.graphics.getHeight()

    -- Merge user config with defaults
    if config then
        PostFX.mergeConfig(config)
    end

    -- Create canvases
    local settings = {format = "rgba8", readable = true}
    mainCanvas = love.graphics.newCanvas(w, h, settings)
    distortionCanvas = love.graphics.newCanvas(w, h, settings)
    brightnessCanvas = love.graphics.newCanvas(w, h, settings)
    blurTempCanvas = love.graphics.newCanvas(w, h, settings)
    bloomCanvas = love.graphics.newCanvas(w, h, settings)
    compositeCanvas = love.graphics.newCanvas(w, h, settings)

    -- Compile shaders
    local function compile(name, source)
        local success, result = pcall(love.graphics.newShader, source)
        if not success then
            print("[PostFX] ERROR compiling " .. name .. ": " .. tostring(result))
            return nil
        end
        return result
    end

    distortionShader = compile("distortion", distortionSource)
    brightnessShader = compile("brightness", brightnessSource)
    blurHorizontalShader = compile("blurHorizontal", blurHorizontalSource)
    blurVerticalShader = compile("blurVertical", blurVerticalSource)
    colorGradeShader = compile("colorGrade", colorGradeSource)
end

--- Merge user configuration with defaults (deep merge)
function PostFX.mergeConfig(userConfig)
    for category, settings in pairs(userConfig) do
        if PostFX.config[category] then
            for key, value in pairs(settings) do
                PostFX.config[category][key] = value
            end
        end
    end
end

--- Recreate canvases on resize
--- @param w number Native width
--- @param h number Native height
function PostFX.resize(w, h)
    local settings = {format = "rgba8", readable = true}
    mainCanvas = love.graphics.newCanvas(w, h, settings)
    distortionCanvas = love.graphics.newCanvas(w, h, settings)
    brightnessCanvas = love.graphics.newCanvas(w, h, settings)
    blurTempCanvas = love.graphics.newCanvas(w, h, settings)
    bloomCanvas = love.graphics.newCanvas(w, h, settings)
    compositeCanvas = love.graphics.newCanvas(w, h, settings)
end

--- Update time uniform
--- @param dt number Delta time
function PostFX.update(dt)
    elapsedTime = love.timer.getTime() - startTime
end

--- Begin rendering scene to canvas
function PostFX.beginScene()
    if not mainCanvas then return end
    love.graphics.setCanvas(mainCanvas)
    love.graphics.clear(0, 0, 0, 1)
end

--- End scene and apply post-processing
--- @param scale number|nil Scale factor (default 1)
--- @param offsetX number|nil X offset for centering (default 0)
--- @param offsetY number|nil Y offset for centering (default 0)
function PostFX.endScene(scale, offsetX, offsetY)
    scale = scale or 1
    offsetX = offsetX or 0
    offsetY = offsetY or 0

    love.graphics.setCanvas()
    love.graphics.setShader()

    if not mainCanvas then return end

    -- Bypass mode: draw without effects
    if PostFX.bypass then
        love.graphics.setColor(1, 1, 1, 1)
        love.graphics.draw(mainCanvas, offsetX, offsetY, 0, scale, scale)
        return
    end

    local canvasW, canvasH = mainCanvas:getDimensions()
    local currentCanvas = mainCanvas

    -- Pass 1: Distortion
    if PostFX.config.distortion.enabled and distortionShader then
        currentCanvas = PostFX._applyDistortion(currentCanvas)
    end

    -- Pass 2: Bloom
    if PostFX.config.bloom.enabled and brightnessShader and blurHorizontalShader and blurVerticalShader then
        currentCanvas = PostFX._applyBloom(currentCanvas, canvasW, canvasH)
    end

    -- Pass 3: Color Grading (outputs to screen)
    if PostFX.config.colorGrade.enabled and colorGradeShader then
        PostFX._applyColorGrade(currentCanvas, scale, offsetX, offsetY)
    else
        love.graphics.setColor(1, 1, 1, 1)
        love.graphics.draw(currentCanvas, offsetX, offsetY, 0, scale, scale)
    end
end

--- Handle keyboard input for runtime tweaking
--- @param key string Key pressed
function PostFX.handleKeyPress(key)
    if key == "f1" then
        PostFX.bypass = not PostFX.bypass
    elseif key == "f2" then
        PostFX.config.distortion.enabled = not PostFX.config.distortion.enabled
    elseif key == "f3" then
        PostFX.config.bloom.enabled = not PostFX.config.bloom.enabled
    elseif key == "f4" then
        PostFX.config.colorGrade.enabled = not PostFX.config.colorGrade.enabled
    end
end

--============================================================================
-- INTERNAL PASSES
--============================================================================

function PostFX._applyDistortion(sourceCanvas)
    if not distortionCanvas then return sourceCanvas end

    love.graphics.setCanvas(distortionCanvas)
    love.graphics.clear(0, 0, 0, 0)
    love.graphics.setColor(1, 1, 1, 1)
    love.graphics.setBlendMode("alpha", "premultiplied")

    love.graphics.setShader(distortionShader)
    distortionShader:send("time", elapsedTime)
    distortionShader:send("strength", PostFX.config.distortion.strength)
    distortionShader:send("speed", PostFX.config.distortion.speed)

    love.graphics.draw(sourceCanvas, 0, 0)

    love.graphics.setShader()
    love.graphics.setBlendMode("alpha")
    love.graphics.setCanvas()

    return distortionCanvas
end

function PostFX._applyBloom(sourceCanvas, w, h)
    local cfg = PostFX.config.bloom

    -- Brightness threshold
    love.graphics.setCanvas(brightnessCanvas)
    love.graphics.clear(0, 0, 0, 1)
    love.graphics.setShader(brightnessShader)
    brightnessShader:send("threshold", cfg.threshold)
    brightnessShader:send("softness", cfg.softness)
    love.graphics.draw(sourceCanvas, 0, 0)

    -- Horizontal blur
    love.graphics.setCanvas(blurTempCanvas)
    love.graphics.clear(0, 0, 0, 1)
    love.graphics.setShader(blurHorizontalShader)
    blurHorizontalShader:send("blur_size", cfg.radius / w)
    love.graphics.draw(brightnessCanvas, 0, 0)

    -- Vertical blur
    love.graphics.setCanvas(bloomCanvas)
    love.graphics.clear(0, 0, 0, 1)
    love.graphics.setShader(blurVerticalShader)
    blurVerticalShader:send("blur_size", cfg.radius / h)
    love.graphics.draw(blurTempCanvas, 0, 0)

    -- Composite
    love.graphics.setCanvas(compositeCanvas)
    love.graphics.clear()
    love.graphics.setShader()
    love.graphics.draw(sourceCanvas, 0, 0)
    love.graphics.setBlendMode("add")
    love.graphics.setColor(cfg.intensity, cfg.intensity, cfg.intensity, 1)
    love.graphics.draw(bloomCanvas, 0, 0)
    love.graphics.setColor(1, 1, 1, 1)
    love.graphics.setBlendMode("alpha")

    love.graphics.setCanvas()
    return compositeCanvas
end

function PostFX._applyColorGrade(sourceCanvas, scale, offsetX, offsetY)
    local cfg = PostFX.config.colorGrade

    love.graphics.setShader(colorGradeShader)
    colorGradeShader:send("saturation", cfg.saturation)
    colorGradeShader:send("contrast", cfg.contrast)
    colorGradeShader:send("warmth", cfg.warmth)
    colorGradeShader:send("vignetteStrength", cfg.vignetteStrength)
    colorGradeShader:send("vignetteRadius", cfg.vignetteRadius)
    colorGradeShader:send("time", elapsedTime)
    colorGradeShader:send("shadowTint", cfg.shadowTint)
    colorGradeShader:send("highlightTint", cfg.highlightTint)

    love.graphics.draw(sourceCanvas, offsetX, offsetY, 0, scale, scale)
    love.graphics.setShader()
end

return PostFX
