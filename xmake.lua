set_project("try")
set_version("0.1.0")

shared = {
    projectdir = os.projectdir(),
    guests = {
        arm64 = {
            arch = "aarch64",
            tarball = "http://mirror.archlinuxarm.org/os/ArchLinuxARM-aarch64-latest.tar.gz",
            kernel = "boot/Image",
            initrd = "boot/initramfs-linux.img"
        }
    }
}

shared.builddir = path.join(shared.projectdir, "build")
shared.cachedir = path.join(shared.projectdir, ".cache")
shared.tarball = path.join(shared.cachedir, "archlinuxarm.tar.gz")
shared.md5file = path.join(shared.cachedir, "archlinuxarm.md5")
shared.rootdir = path.join(shared.cachedir, "root")
shared.rawfile = path.join(shared.builddir, "rootfs.raw")
shared.logfile = path.join(shared.builddir, "guest.log")
shared.statedir = os.getenv("TRY_STATE_DIR") or path.join(shared.projectdir, "state")
shared.settingsfile = path.join(shared.statedir, "settings")
shared.diskfile = path.join(shared.statedir, "rootfs.qcow2")
shared.provisionstamp = path.join(shared.builddir, "provision.stamp")
shared.disksize = "32G"
shared.dotfiles = os.getenv("HOME") and path.join(os.getenv("HOME"), "dotfiles") or nil
shared.firstbootdir = path.join(shared.projectdir, "guest/firstBoot")
shared.firstbootfile = path.join(shared.builddir, "guest/firstboot")
shared.trydisplaydir = path.join(shared.projectdir, "guest/tryDisplay")
shared.trydisplayfile = path.join(shared.builddir, "guest/tryDisplay")
shared.guest_arch = "arm64"

shared.qemudir = path.join(shared.cachedir, "qemu")
shared.qemurelease = "https://github.com/milesbuckton/homebrew-qemu-virgl/releases/download/latest"
shared.qemubottles = {
    {file = "qemu-master.arm64_tahoe.tgz",
     release = "https://github.com/yumedots/qemu/releases/download/qemu-2e00971192968c013dc0ada9e5babe07769bb966",
     sha256 = "8b48c8134bb6618c6a21f55d67a666952a65ee01bbe31f3b404bc2d28c9323fa"},
    {file = "virglrenderer-main.arm64_tahoe.bottle.1.tar.gz", opt = "virglrenderer",
     sha256 = "cc31b53d76cecc4d3e8b212da03e80dcd6628f015f8ec05823c7c2e9747221b2"},
    {file = "libangle-main.20260909.3e88857d9.arm64_tahoe.bottle.1.tar.gz", opt = "libangle",
     sha256 = "a3a947b6bff1978edb11e3e8949ae6dc544c894faeb1eaba4de3b6293e0c7cab"},
    {file = "libepoxy-angle-master.arm64_tahoe.bottle.1.tar.gz", opt = "libepoxy-angle",
     sha256 = "dcfa0103caa7293e517239a2e860dd44e45c49c3f017ae89acb7ca40c9f58f5a"}
}

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

function shared.settings(os, io)
    local gigabytes = math.floor((os.meminfo("totalsize") or 0) / 1024)
    local values = {
        cores = tostring(math.min(8, os.cpuinfo().ncpu)),
        mem = gigabytes >= 16 and "8G" or "4G",
        audio = os.host() == "macosx" and "on" or "off"
    }
    if os.isfile(shared.settingsfile) then
        for line in io.readfile(shared.settingsfile):gmatch("[^\r\n]+") do
            local key, value = line:match("^([%w_]+)=(.+)$")
            if key and values[key] then
                values[key] = value:trim()
            end
        end
    end
    return values
end

function shared.state(os)
    os.mkdir(shared.statedir)
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


function shared.guest()
    local g = shared.guests[os.arch()]
    if not g then
        os.raise("no guest image for host arch " .. os.arch() .. " yet")
    end
    return g
end

function shared.remote_md5(url, os)
    return os.iorunv("sh", {"-c", "curl -fsSL " .. url .. ".md5 2>/dev/null || true"}):split("%s")[1]
end

function shared.mke2fs(find_tool)
    local tool = find_tool("mke2fs") or find_tool("mkfs.ext4")
    if tool then
        return tool.program
    end
    for _, p in ipairs({"/opt/homebrew/opt/e2fsprogs/sbin/mke2fs", "/usr/sbin/mke2fs", "/sbin/mke2fs", "/usr/bin/mke2fs"}) do
        if os.isfile(p) then
            return p
        end
    end
end

function shared.go(os, find_tool)
    local tool = find_tool and find_tool("go") or nil
    if tool then
        return tool.program
    end

    local host = os.host() == "macosx" and "darwin" or "linux"
    local arch = os.arch() == "x86_64" and "amd64" or os.arch()
    local toolchain_dir = path.join(shared.cachedir, "toolchains/go")
    local go = path.join(toolchain_dir, "go/bin/go")
    if os.isfile(go) then
        return go
    end

    local version = os.iorunv("curl", {"-fsSL", "https://go.dev/VERSION?m=text"}):split("\n")[1]
    local archive_url = "https://go.dev/dl/" .. version .. "." .. host .. "-" .. arch .. ".tar.gz"

    os.mkdir(toolchain_dir)
    local archive = path.join(toolchain_dir, path.filename(archive_url))
    print("download Go toolchain " .. archive_url)
    os.execv("curl", {"-fL", "--retry", "3", "-o", archive, archive_url})
    os.execv("tar", {"-xzf", archive, "-C", toolchain_dir})
    os.rm(archive)
    if not os.isfile(go) then
        os.raise("Go toolchain extraction failed")
    end
    return go
end

function shared.build_guest_go(os, dir, target, find_tool)
    local go = shared.go(os, find_tool)
    os.mkdir(path.directory(target))
    os.execv(go, {"build", "-trimpath", "-ldflags=-s -w", "-o", target, "."}, {
        curdir = dir,
        envs = {
            GOOS = "linux",
            GOARCH = shared.guest_arch,
            CGO_ENABLED = "0",
            GO111MODULE = "on",
            GOCACHE = path.join(shared.cachedir, "go-build"),
            GOMODCACHE = path.join(shared.cachedir, "go-mod"),
            GOPATH = path.join(shared.cachedir, "go-path")
        }
    })
    os.execv("chmod", {"755", target})
    return target
end

function shared.build_firstboot(os, target, find_tool)
    return shared.build_guest_go(os, shared.firstbootdir, target or shared.firstbootfile, find_tool)
end

function shared.build_trydisplay(os, target, find_tool)
    return shared.build_guest_go(os, shared.trydisplaydir, target or shared.trydisplayfile, find_tool)
end

function shared.pigz(os)
    local bin = path.join(shared.cachedir, "bin/pigz")
    if os.isfile(bin) then
        return bin
    end
    if not os.isdir(shared.cachedir) then
        os.mkdir(shared.cachedir)
    end
    local src = path.join(shared.cachedir, "pigz.tar.gz")
    local dir = path.join(shared.cachedir, "pigz")
    print("building pigz (parallel gzip) into .cache/bin, no install needed")
    os.execv("sh", {"-c", "curl -fsSL -o " .. src .. " https://zlib.net/pigz/pigz.tar.gz" ..
        " && tar -xzf " .. src .. " -C " .. shared.cachedir ..
        " && mkdir -p " .. path.directory(bin) ..
        " && make -s -C " .. dir .. " pigz && cp " .. path.join(dir, "pigz") .. " " .. bin .. " || true"})
    if os.isfile(bin) then
        return bin
    end
    print("pigz build failed, falling back to tar")
    return nil
end

function shared.fetch_latest(g, os, io, force)
    os.mkdir(shared.cachedir)
    if os.isfile(shared.tarball) and os.filesize(shared.tarball) > 0 and not force then
        local cached = os.isfile(shared.md5file) and io.readfile(shared.md5file):trim() or "?"
        print("tarball cached " .. cached)
        return
    end
    local want = shared.remote_md5(g.tarball, os)
    if want and os.isfile(shared.tarball) and hash.md5(shared.tarball) == want then
        print("tarball up to date " .. want)
        return
    end
    local started = os.time()
    print("download (8 parallel ranges) " .. g.tarball)
    local parts = path.join(shared.cachedir, "parts")
    if os.isdir(parts) then
        os.rmdir(parts)
    end
    os.mkdir(parts)
    os.execv("sh", {"-c", "url=" .. g.tarball .. "; out=" .. shared.tarball .. "; parts=" .. parts .. [[
; size=$(curl -fsSLI "$url" | tr -d '
' | awk 'tolower($1)=="content-length:"{print $2}' | tail -1)
n=8
[ "${size:-0}" -gt 8000000 ] || n=1
chunk=$(( (${size:-0} + n - 1) / n ))
i=0
while [ "$i" -lt "$n" ]; do
    from=$(( i * chunk ))
    to=$(( from + chunk - 1 ))
    [ "$to" -ge "${size:-0}" ] && to=$(( ${size:-0} - 1 ))
    range="$from-$to"
    [ "$n" -eq 1 ] && range="$from-"
    curl -fsSL -r "$range" -o "$parts/part.$i" "$url" &
    i=$(( i + 1 ))
done
want=$(( ${size:-0} / 1024 ))
i=0
while [ "$i" -lt 900 ]; do
    got=$(du -sk "$parts" 2>/dev/null | awk '{print $1}')
    printf '\r%s%% (%s MB)' "$(( got * 100 / (want + 1) ))" "$(( got / 1024 ))" > /dev/tty 2>/dev/null || true
    [ "$got" -ge "$want" ] && break
    sleep 1
    i=$(( i + 1 ))
done
wait
printf '\r\n' > /dev/tty 2>/dev/null || true
cat "$parts"/part.* > "$out"
rm -rf "$parts"
]]})
    local actual = hash.md5(shared.tarball)
    if want and actual ~= want then
        print("parallel download incomplete, retrying in one stream")
        os.execv("curl", {"-fL", "--progress-bar", "--retry", "3", "-o", shared.tarball, g.tarball})
        actual = hash.md5(shared.tarball)
    end
    if want and actual ~= want then
        os.raise("tarball md5 mismatch: mirror says " .. want .. ", got " .. actual)
    end
    io.writefile(shared.md5file, actual .. "\n")
    print("tarball ok " .. actual .. " in " .. (os.time() - started) .. "s")
end

function shared.provision_key(os)
    local stamp = shared.tarball .. " " .. tostring(os.filesize(shared.tarball)) .. " " .. tostring(os.mtime(shared.tarball))
    local lines = {shared.disksize, stamp}
    local patterns = {path.join(shared.projectdir, "guest/**")}
    if shared.dotfiles then
        patterns[#patterns + 1] = path.join(shared.dotfiles, "**")
    end
    for _, pattern in ipairs(patterns) do
        for _, file in ipairs(os.files(pattern)) do
            lines[#lines + 1] = file .. " " .. tostring(os.filesize(file)) .. " " .. tostring(os.mtime(file))
        end
    end
    table.sort(lines)
    return table.concat(lines, "\n")
end

function shared.prepare_guest(g, os, find_tool, io)
    local total = os.time()
    if not os.isfile(shared.tarball) then
        os.raise("tarball missing, run: xmake fetch")
    end
    if not os.isdir(shared.builddir) then
        os.mkdir(shared.builddir)
    end
    os.mkdir(shared.cachedir)
    shared.state(os)
    local key = shared.provision_key(os)
    local stamp = os.isfile(shared.provisionstamp) and io.readfile(shared.provisionstamp) or ""
    if os.isfile(shared.diskfile) and stamp == key then
        print("disk kept " .. shared.diskfile)
        return
    end
    local firstboot = shared.build_firstboot(os, nil, find_tool)
    local trydisplay = shared.build_trydisplay(os, nil, find_tool)
    os.mkdir(shared.rootdir)
    if not os.isfile(path.join(shared.rootdir, "etc/passwd")) then
        local started = os.time()
        local pigz = shared.pigz(os)
        local extract
        if pigz then
            print("extract (pigz, all cores) " .. shared.tarball)
            extract = pigz .. " -dc " .. shared.tarball .. " | tar -xpf - -C " .. shared.rootdir
        else
            print("extract (tar) " .. shared.tarball)
            extract = "tar -xpf " .. shared.tarball .. " -C " .. shared.rootdir
        end
        os.execv("sh", {"-c", extract .. " 2>/dev/null || true"})
        if not os.isfile(path.join(shared.rootdir, "etc/passwd")) then
            os.raise("extract failed, the tarball is empty or broken, run: xmake fetch")
        end
        os.execv("chmod", {"-R", "u+rwX", shared.rootdir})
        print("extracted in " .. (os.time() - started) .. "s")
    else
        print("tree cached " .. shared.rootdir)
    end

    local prov = path.join(shared.rootdir, "usr/local/lib/try")
    if os.isdir(prov) then
        os.rmdir(prov)
    end
    os.mkdir(prov)
    os.cp(firstboot, path.join(prov, "firstboot"))
    os.cp(trydisplay, path.join(prov, "tryDisplay"))
    os.cp(path.join(shared.projectdir, "guest/packages.txt"), prov)
    os.cp(path.join(shared.projectdir, "guest/Xsetup"), path.join(prov, "Xsetup"))
    if not shared.dotfiles or not os.isdir(shared.dotfiles) then
        os.raise("dotfiles source missing or submodule not populated: " .. tostring(shared.dotfiles))
    end
    os.cp(path.join(shared.dotfiles, "*"), path.join(prov, "dotfiles"))
    os.execv("chmod", {"755", path.join(prov, "firstboot"), path.join(prov, "tryDisplay"), path.join(prov, "Xsetup")})
    os.cp(path.join(shared.projectdir, "guest/firstBoot.service"), path.join(shared.rootdir, "etc/systemd/system/firstBoot.service"))
    local wants = path.join(shared.rootdir, "etc/systemd/system/multi-user.target.wants")
    os.mkdir(wants)
    os.execv("ln", {"-sf", "/etc/systemd/system/firstBoot.service", path.join(wants, "firstBoot.service")})
    os.cp(path.join(shared.projectdir, "guest/tryDisplay.service"), path.join(shared.rootdir, "etc/systemd/system/tryDisplay.service"))
    local graphical_wants = path.join(shared.rootdir, "etc/systemd/system/graphical.target.wants")
    os.mkdir(graphical_wants)
    os.execv("ln", {"-sf", "/etc/systemd/system/tryDisplay.service", path.join(graphical_wants, "tryDisplay.service")})

    if os.isfile(shared.diskfile) then
        print("disk kept " .. shared.diskfile)
        io.writefile(shared.provisionstamp, key)
        return
    end
    local mke2fs = shared.mke2fs(find_tool)
    if not mke2fs then
        os.raise("mke2fs not found, install e2fsprogs")
    end
    local started = os.time()
    print("create " .. shared.rawfile .. " " .. shared.disksize)
    os.rm(shared.rawfile)
    local qemu_img = shared.qemu_img(os, find_tool)
    os.execv(qemu_img, {"create", "-f", "raw", shared.rawfile, shared.disksize})
    os.execv(mke2fs, {"-t", "ext4", "-F", "-E", "lazy_itable_init=1,lazy_journal_init=1", "-J", "size=64", "-d", shared.rootdir, shared.rawfile})
    print("raw built in " .. (os.time() - started) .. "s")
    local convert_started = os.time()
    print("convert " .. shared.rawfile .. " -> " .. shared.diskfile)
    os.execv(qemu_img, {"convert", "-O", "qcow2", "-m", "8", "-W", shared.rawfile, shared.diskfile})
    os.rm(shared.rawfile)
    print("disk ready in " .. (os.time() - convert_started) .. "s")
    print("size " .. os.iorunv("sh", {"-c", "ls -lh " .. shared.diskfile .. " | awk '{print $5}'"}):trim())
    print("guest image ready in " .. (os.time() - total) .. "s total")
    io.writefile(shared.provisionstamp, key)
end

includes("guest")
includes("qemu")
includes("ui")

task("clean")
on_run(function ()
    import("core.base.option")
    os.rmdir(shared.builddir)
    print("deleted " .. shared.builddir)
    if option.get("cache") then
        os.rmdir(shared.cachedir)
        print("deleted " .. shared.cachedir)
    else
        print("kept " .. shared.cachedir .. " (tarball + tree, drop with: xmake clean --cache)")
        print("kept " .. shared.statedir .. " (guest disk + settings, drop with: xmake reset)")
    end
end)
set_menu {
    usage = "xmake clean [--cache]",
    description = "Delete build/, keeping the download and tree cache in .cache/",
    options = {
        {nil, "cache", "k", nil, "Also delete .cache/ (tarball + extracted tree)"}
    }
}
