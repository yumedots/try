set_project("try")
set_version("0.1.0")

shared = {
    projectdir = os.projectdir(),
    guests = {
        arm64 = {
            arch = "aarch64",
            tarball = "https://ca.us.mirror.archlinuxarm.org/os/ArchLinuxARM-aarch64-latest.tar.gz",
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
shared.diskfile = path.join(shared.builddir, "rootfs.qcow2")
shared.logfile = path.join(shared.builddir, "guest.log")
shared.disksize = "32G"
shared.dotfiles = os.getenv("HOME") and path.join(os.getenv("HOME"), "dotfiles") or nil
shared.firstbootdir = path.join(shared.projectdir, "guest/firstBoot")
shared.firstbootfile = path.join(shared.builddir, "guest/firstboot")
shared.trydisplaydir = path.join(shared.projectdir, "guest/tryDisplay")
shared.trydisplayfile = path.join(shared.builddir, "guest/tryDisplay")
shared.guest_arch = "arm64"


function shared.guest()
    local g = shared.guests[os.arch()]
    if not g then
        os.raise("no guest image for host arch " .. os.arch() .. " yet")
    end
    return g
end

function shared.remote_md5(url, os)
    return os.iorunv("curl", {"-fsSL", url .. ".md5"}):split("%s")[1]
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
    if os.isfile(shared.tarball) and not force then
        print("tarball cached " .. hash.md5(shared.tarball))
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

function shared.prepare_guest(g, os, find_tool, io)
    local total = os.time()
    if not os.isfile(shared.tarball) then
        os.raise("tarball missing, run: xmake fetch")
    end
    if not os.isdir(shared.builddir) then
        os.mkdir(shared.builddir)
    end
    os.mkdir(shared.cachedir)
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
            os.raise("extract failed, rm -rf " .. shared.rootdir .. " and retry")
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
    if not shared.dotfiles or not os.isdir(shared.dotfiles) then
        os.raise("dotfiles source missing or submodule not populated: " .. tostring(shared.dotfiles))
    end
    os.cp(path.join(shared.dotfiles, "*"), path.join(prov, "dotfiles"))
    os.execv("chmod", {"755", path.join(prov, "firstboot"), path.join(prov, "tryDisplay")})
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
        return
    end
    local mke2fs = shared.mke2fs(find_tool)
    if not mke2fs then
        os.raise("mke2fs not found, install e2fsprogs")
    end
    local started = os.time()
    print("create " .. shared.rawfile .. " " .. shared.disksize)
    os.rm(shared.rawfile)
    os.execv("qemu-img", {"create", "-f", "raw", shared.rawfile, shared.disksize})
    os.execv(mke2fs, {"-t", "ext4", "-F", "-d", shared.rootdir, shared.rawfile})
    print("raw built in " .. (os.time() - started) .. "s")
    local convert_started = os.time()
    print("convert " .. shared.rawfile .. " -> " .. shared.diskfile)
    os.execv("qemu-img", {"convert", "-O", "qcow2", shared.rawfile, shared.diskfile})
    os.rm(shared.rawfile)
    print("disk ready in " .. (os.time() - convert_started) .. "s")
    print("size " .. os.iorunv("sh", {"-c", "ls -lh " .. shared.diskfile .. " | awk '{print $5}'"}):trim())
    print("guest image ready in " .. (os.time() - total) .. "s total")
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
    end
end)
set_menu {
    usage = "xmake clean [--cache]",
    description = "Delete build/, keeping the download and tree cache in .cache/",
    options = {
        {nil, "cache", "k", nil, "Also delete .cache/ (tarball + extracted tree)"}
    }
}
