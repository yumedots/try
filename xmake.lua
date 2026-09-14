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
shared.tarball = path.join(shared.builddir, "archlinuxarm.tar.gz")
shared.md5file = path.join(shared.builddir, "archlinuxarm.md5")
shared.rootdir = path.join(shared.builddir, "root")
shared.rawfile = path.join(shared.builddir, "rootfs.raw")
shared.diskfile = path.join(shared.builddir, "rootfs.qcow2")
shared.logfile = path.join(shared.builddir, "guest.log")
shared.disksize = "32G"

function shared.guest()
    local g = shared.guests[os.arch()]
    if not g then
        assert(false, "no guest image for host arch " .. os.arch() .. " yet")
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

function shared.fetch_latest(g, os, download, io)
    os.mkdir(shared.builddir)
    local want = shared.remote_md5(g.tarball, os)
    if want and os.isfile(shared.tarball) and hash.md5(shared.tarball) == want then
        print("tarball up to date " .. want)
        return
    end
    print("download " .. g.tarball)
    download(g.tarball, shared.tarball, {continue = true, insecure_fallback = true})
    local actual = hash.md5(shared.tarball)
    if want and actual ~= want then
        assert(false, "tarball md5 mismatch: mirror says " .. want .. ", got " .. actual)
    end
    io.writefile(shared.md5file, actual .. "\n")
    print("tarball ok " .. actual)
end

function shared.prepare_guest(g, os, find_tool)
    if not os.isfile(shared.tarball) then
        assert(false, "tarball missing, run: xmake fetch")
    end
    os.mkdir(shared.builddir)
    os.mkdir(shared.rootdir)
    if not os.isfile(path.join(shared.rootdir, "etc/passwd")) then
        print("extract " .. shared.tarball)
        os.execv("sh", {"-c", "tar -xpf " .. shared.tarball .. " -C " .. shared.rootdir .. " || true"})
        if not os.isfile(path.join(shared.rootdir, "etc/passwd")) then
            assert(false, "extract failed, rm -rf " .. shared.rootdir .. " and retry")
        end
    end
    os.execv("chmod", {"-R", "u+rwX", shared.rootdir})

    local prov = path.join(shared.rootdir, "usr/local/lib/try")
    if os.isdir(prov) then
        os.rmdir(prov)
    end
    os.mkdir(prov)
    os.cp(path.join(shared.projectdir, "guest/firstBoot.sh"), prov)
    os.cp(path.join(shared.projectdir, "guest/packages.txt"), prov)
    os.execv("chmod", {"755", path.join(prov, "firstBoot.sh")})
    os.cp(path.join(shared.projectdir, "guest/firstBoot.service"), path.join(shared.rootdir, "etc/systemd/system/firstBoot.service"))
    local wants = path.join(shared.rootdir, "etc/systemd/system/multi-user.target.wants")
    os.mkdir(wants)
    os.execv("ln", {"-sf", "/etc/systemd/system/firstBoot.service", path.join(wants, "firstBoot.service")})

    if os.isfile(shared.diskfile) then
        print("disk kept " .. shared.diskfile)
        return
    end
    if not os.isfile(shared.rawfile) then
        local mke2fs = shared.mke2fs(find_tool)
        if not mke2fs then
            assert(false, "mke2fs not found, install e2fsprogs")
        end
        print("create " .. shared.rawfile .. " " .. shared.disksize)
        os.execv("qemu-img", {"create", "-f", "raw", shared.rawfile, shared.disksize})
        os.execv(mke2fs, {"-t", "ext4", "-F", "-d", shared.rootdir, shared.rawfile})
    end
    print("convert " .. shared.rawfile .. " -> " .. shared.diskfile)
    os.execv("qemu-img", {"convert", "-O", "qcow2", shared.rawfile, shared.diskfile})
    os.rm(shared.rawfile)
end

includes("guest")
includes("qemu")

task("clean")
on_run(function ()
    os.rmdir(shared.builddir)
end)
set_menu {
    usage = "xmake clean",
    description = "Delete build/"
}
