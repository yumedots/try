function shared.qemu_keg(os)
    if os.host() ~= "macosx" or os.arch() ~= "arm64" then
        return nil
    end
    if tonumber(os.iorunv("sw_vers", {"-productVersion"}):match("^(%d+)")) < 26 then
        return nil
    end
    return shared.qemudir
end

function shared.qemu_program(os, find_tool)
    local keg = shared.qemu_keg(os)
    local program = keg and path.join(keg, "bin/qemu-system-" .. shared.guest().arch) or nil
    if program and os.isfile(program) then
        return program
    end
    local tool = find_tool("qemu-system-" .. shared.guest().arch)
    return tool and tool.program
end

function shared.qemu_img(os, find_tool)
    local keg = shared.qemu_keg(os)
    local program = keg and path.join(keg, "bin/qemu-img") or nil
    if not program or not os.isfile(program) then
        local tool = find_tool and find_tool("qemu-img") or nil
        program = tool and tool.program
    end
    return program or "qemu-img"
end

function shared.virtualization(os, io, program)
    if not program or os.host() ~= "macosx" or os.arch() ~= "arm64" then
        return false
    end
    if tonumber(os.iorunv("sw_vers", {"-productVersion"}):match("^(%d+)")) < 26 then
        return false
    end
    local stamp = path.join(shared.cachedir, "virtualization")
    if os.isfile(stamp) then
        return io.readfile(stamp):trim() == "on"
    end
    local ok = os.iorunv("sh", {"-c", "printf 'quit\\n' | '" .. program ..
        "' -nodefaults -machine virt,accel=hvf,virtualization=on -cpu host -display none -S -monitor stdio >/dev/null 2>&1 && echo on || echo off"}):trim()
    print("nested virtualization " .. ok)
    os.mkdir(shared.cachedir)
    io.writefile(stamp, ok .. "\n")
    return ok == "on"
end

function shared.qemu_sharedir(os)
    local keg = shared.qemu_keg(os)
    local dir = keg and path.join(keg, "share/qemu") or nil
    return dir and os.isdir(dir) and dir or nil
end

function shared.qemu_env(os)
    local keg = shared.qemu_keg(os)
    if not keg then
        return nil
    end
    local paths = {}
    for _, dir in ipairs(os.dirs(path.join(keg, "opt", "*")) or {}) do
        table.insert(paths, path.join(dir, "lib"))
    end
    return {DYLD_FALLBACK_LIBRARY_PATH = table.concat(paths, ":")}
end

function shared.qemu_home(os)
    local brew = os.iorunv("sh", {"-c", "command -v brew || true"}):trim()
    if brew ~= "" then
        return os.iorunv(brew, {"--prefix"}):trim()
    end
    return "/opt/homebrew"
end

function shared.qemu_link(os, keg, dependency)
    local home = shared.qemu_home(os)
    local rest = dependency:match("^@@HOMEBREW_PREFIX@@(/opt/.+)") or dependency:match("^" .. home .. "(/opt/.+)")
    local name = rest and rest:match("^/opt/([^/]+)/")
    if not rest then
        return dependency
    end
    return (name and os.isdir(path.join(keg, "opt", name)) and keg or home) .. rest
end

function shared.qemu_patch(os, keg, binary)
    local home = shared.qemu_home(os)
    local changes = {}
    for line in os.iorunv("otool", {"-L", binary}):gmatch("[^\r\n]+") do
        local dependency = line:match("(@@HOMEBREW_PREFIX@@[^%s]+)") or line:match("(" .. home .. "/opt/[^%s]+)")
        if dependency then
            local target = shared.qemu_link(os, keg, dependency)
            if target ~= dependency then
                table.insert(changes, "install_name_tool -change " .. dependency .. " " .. target .. " " .. binary .. " 2>/dev/null")
            end
        end
    end
    if #changes > 0 then
        table.insert(changes, "codesign --force --sign - --preserve-metadata=entitlements " .. binary .. " 2>/dev/null")
        os.execv("sh", {"-c", table.concat(changes, "\n") .. "\nexit 0"})
    end
end

function shared.qemu_images(os, keg)
    local files = {}
    local roots = {keg}
    table.join2(roots, os.dirs(path.join(keg, "opt", "*")) or {})
    for _, root in ipairs(roots) do
        for _, pattern in ipairs({"bin/*", "lib/*.dylib", "libexec/*"}) do
            table.join2(files, os.files(path.join(root, pattern)) or {})
        end
    end
    return files
end

function shared.qemu_missing(os, keg)
    local home = shared.qemu_home(os)
    local missing, seen = {}, {}
    for _, binary in ipairs(shared.qemu_images(os, keg)) do
        for line in os.iorunv("otool", {"-L", binary}):gmatch("[^\r\n]+") do
            local dependency = line:match("^%s*(" .. home .. "/opt/[^%s]+)")
            local name = dependency and dependency:match("^" .. home .. "/opt/([^/]+)/")
            if name and not os.isfile(dependency) and not seen[name] then
                seen[name] = true
                table.insert(missing, name)
            end
        end
    end
    return missing
end

function shared.qemu_dependency(os, keg, name)
    local archive = path.join(shared.cachedir, "homebrew-" .. name .. ".tar.gz")
    local dest = path.join(keg, "opt", name)
    if os.isdir(dest) then
        os.rmdir(dest)
    end
    local sha256
    if os.isfile(archive) then
        sha256 = hash.sha256(archive)
    end
    if not sha256 then
        local json = os.iorunv("curl", {"-fsSL", "https://formulae.brew.sh/api/formula/" .. name .. ".json"})
        local bottle = json:match('"arm64_tahoe":%s*{(.-)}') or json:match('"arm64_golden_gate":%s*{(.-)}')
        local url = bottle and bottle:match('"url":"([^"]+)"') or nil
        sha256 = bottle and bottle:match('"sha256":"([^"]+)"') or nil
        if not url or not sha256 then
            os.raise("no bottle for homebrew formula " .. name .. ", install it with brew")
        end
        print("download homebrew/" .. name .. " " .. sha256)
        os.execv("sh", {"-c", "token=$(curl -fsSL 'https://ghcr.io/token?service=ghcr.io&scope=repository:homebrew/core/" ..
            name .. ":pull' | sed -n 's/.*\"token\": *\"\\([^\"]*\\)\".*/\\1/p')\n" ..
            "curl -fsSL -H \"Authorization: Bearer $token\" -o " .. archive .. " '" .. url .. "'"})
        if hash.sha256(archive) ~= sha256 then
            os.raise("bottle mismatch homebrew/" .. name)
        end
    end
    os.mkdir(dest)
    os.execv("tar", {"-xzf", archive, "-C", dest, "--strip-components=2"})
end

function shared.qemu_fetch(os, io, force)
    if os.host() ~= "macosx" then
        return nil
    end
    local keg = shared.qemu_keg(os)
    if not keg then
        os.raise("the qemu-virgl bottle is arm64 tahoe (macOS 26) or newer only: " .. os.host() .. "/" .. os.arch() ..
            ", use a QEMU from PATH with --no-gl")
    end
    local lines = {}
    for _, bottle in ipairs(shared.qemubottles) do
        table.insert(lines, bottle.sha256 .. "  " .. bottle.file)
    end
    local pins = table.concat(lines, "\n") .. "\n"
    local stamp = path.join(keg, ".sha256")
    if not force and os.isfile(stamp) and io.readfile(stamp) == pins then
        print("qemu-virgl cached " .. keg)
        return keg
    end
    os.mkdir(shared.cachedir)
    for _, bottle in ipairs(shared.qemubottles) do
        local archive = path.join(shared.cachedir, bottle.file)
        if not os.isfile(archive) or hash.sha256(archive) ~= bottle.sha256 then
            print("download " .. bottle.file)
            os.execv("curl", {"-fL", "--retry", "3", "-o", archive, (bottle.release or shared.qemurelease) .. "/" .. bottle.file})
        end
        local actual = hash.sha256(archive)
        if actual ~= bottle.sha256 then
            os.raise("bottle mismatch " .. bottle.file .. ": want " .. bottle.sha256 .. ", got " .. actual)
        end
        local dest = bottle.opt and path.join(keg, "opt", bottle.opt) or keg
        if bottle.opt and os.isdir(dest) then
            os.rmdir(dest)
        end
        os.mkdir(dest)
        os.execv("tar", {"-xzf", archive, "-C", dest, "--strip-components=2"})
    end
    for _, binary in ipairs(shared.qemu_images(os, keg)) do
        shared.qemu_patch(os, keg, binary)
    end
    for round = 1, 8 do
        local missing = shared.qemu_missing(os, keg)
        if #missing == 0 then
            break
        end
        if round == 8 then
            print("unresolved dependencies: " .. table.concat(missing, " "))
            break
        end
        for _, name in ipairs(missing) do
            shared.qemu_dependency(os, keg, name)
        end
        for _, binary in ipairs(shared.qemu_images(os, keg)) do
            shared.qemu_patch(os, keg, binary)
        end
    end
    io.writefile(stamp, pins)
    print("qemu-virgl ready " .. path.join(keg, "bin/qemu-system-" .. shared.guest().arch))
    return keg
end
