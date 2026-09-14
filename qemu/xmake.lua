local shared = shared

local function qemu_program(find_tool)
    local tool = find_tool("qemu-system-" .. shared.guest().arch)
    return tool and tool.program
end

local function qemu_args(option)
    local g = shared.guest()
    local accel = os.host() == "macosx" and "hvf" or "kvm"
    local argv = {
        "-machine", "virt,accel=" .. accel .. ",highmem=on",
        "-cpu", "host",
        "-smp", option.get("cpus") or "8",
        "-m", option.get("mem") or "8G",
        "-kernel", path.join(shared.rootdir, g.kernel),
        "-initrd", path.join(shared.rootdir, g.initrd),
        "-append", "root=/dev/vda rw console=tty0 console=ttyAMA0",
        "-drive", "if=virtio,format=qcow2,file=" .. shared.diskfile,
        "-device", "virtio-gpu-pci",
        "-device", "virtio-rng-pci",
        "-device", "qemu-xhci",
        "-device", "usb-kbd",
        "-device", "usb-tablet",
        "-netdev", "user,id=net0,hostfwd=tcp::2222-:22",
        "-device", "virtio-net-pci,netdev=net0",
        "-virtfs", "local,path=" .. (option.get("share") or shared.projectdir) .. ",mount_tag=share,security_model=mapped-xattr",
        "-serial", "file:" .. shared.logfile,
        "-no-reboot"
    }
    if os.host() == "macosx" then
        table.join2(argv, {"-display", "cocoa"})
    end
    return argv
end

task("doctor")
on_run(function ()
    import("core.base.option")
    import("lib.detect.find_tool")
    local g = shared.guest()
    local program = qemu_program(find_tool)
    local mke2fs = shared.mke2fs(find_tool)
    print("host      " .. os.host() .. "/" .. os.arch())
    print("guest     " .. g.arch)
    if program then
        print("qemu      " .. program .. " " .. os.iorunv(program, {"--version"}):split("\n")[1])
    else
        print("qemu      missing qemu-system-" .. g.arch)
    end
    print("mke2fs    " .. (mke2fs or "missing e2fsprogs"))
    if os.isfile(shared.tarball) then
        local local_md5 = hash.md5(shared.tarball)
        local want = shared.remote_md5(g.tarball, os)
        print("tarball   " .. local_md5 .. (want == nil and "" or (want == local_md5 and " up to date" or " stale, rerun xmake fetch")))
    else
        print("tarball   missing, run xmake fetch")
    end
    print("rootfs    " .. (os.isfile(path.join(shared.rootdir, "etc/passwd")) and "extracted" or "run xmake build disk"))
    if os.isfile(shared.rawfile) then
        print("disk      raw " .. shared.rawfile .. " pending, run xmake build disk to convert")
    elseif os.isfile(shared.diskfile) then
        local snapshots = os.iorunv("qemu-img", {"snapshot", "-l", shared.diskfile}):trim()
        print("disk      qcow2" .. (snapshots ~= "" and "\n" .. snapshots or ""))
    else
        print("disk      missing, run xmake build disk")
    end
    print("share     " .. (option.get("share") or shared.projectdir))
    print("log       " .. shared.logfile)
    if program and os.isfile(shared.diskfile) then
        print("run       " .. program .. " " .. table.concat(qemu_args(option), " "))
    end
end)
set_menu {
    usage = "xmake doctor",
    description = "Show host, tools, tarball, disk, snapshots and share state"
}

task("run")
on_run(function ()
    import("core.base.option")
    import("lib.detect.find_tool")
    local g = shared.guest()
    local program = qemu_program(find_tool)
    if not program then
        assert(false, "qemu-system-" .. g.arch .. " is not in PATH")
    end
    if not os.isfile(shared.diskfile) then
        import("net.http.download")
        shared.fetch_latest(g, os, download, io)
        shared.prepare_guest(g, os, find_tool)
    end
    if not os.isfile(path.join(shared.rootdir, g.kernel)) then
        assert(false, "guest kernel missing, run: xmake build disk")
    end
    local argv = qemu_args(option)
    if option.get("dry-run") then
        print(program .. " " .. table.concat(argv, " "))
        return
    end
    os.execv(program, argv)
end)
set_menu {
    usage = "xmake run [options]",
    description = "Boot the guest in QEMU",
    options = {
        {nil, "share", "kv", nil, "Host folder to share over 9p (default: this repo)"},
        {nil, "mem", "kv", "8G", "Guest RAM"},
        {nil, "cpus", "kv", "8", "Guest cores"},
        {"n", "dry-run", "k", nil, "Print the QEMU command instead of running it"}
    }
}

task("snapshot")
on_run(function ()
    import("core.base.option")
    local name = option.get("name")
    if not name then
        assert(false, "usage: xmake snapshot --name=<name>")
    end
    os.execv("qemu-img", {"snapshot", "-c", name, shared.diskfile})
    print(os.iorunv("qemu-img", {"snapshot", "-l", shared.diskfile}))
end)
set_menu {
    usage = "xmake snapshot --name=<name>",
    description = "Save a snapshot of the guest disk (shut the VM down first)",
    options = {
        {nil, "name", "kv", nil, "Snapshot name"}
    }
}

task("restore")
on_run(function ()
    import("core.base.option")
    local name = option.get("name")
    if not name then
        assert(false, "usage: xmake restore --name=<name>")
    end
    os.execv("qemu-img", {"snapshot", "-a", name, shared.diskfile})
    print("restored " .. name)
end)
set_menu {
    usage = "xmake restore --name=<name>",
    description = "Roll the guest disk back to a snapshot (shut the VM down first)",
    options = {
        {nil, "name", "kv", nil, "Snapshot name"}
    }
}
