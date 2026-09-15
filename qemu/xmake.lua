local shared = shared

local function qemu_program(find_tool)
    local tool = find_tool("qemu-system-" .. shared.guest().arch)
    return tool and tool.program
end

local function qemu_args(option, os)
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
        "-device", "virtio-gpu-pci,xres=" .. (option.get("width") or "2560") .. ",yres=" .. (option.get("height") or "1440"),
        "-device", "virtio-rng-pci",
        "-device", "qemu-xhci",
        "-device", "usb-kbd",
        "-device", "usb-tablet",
        "-netdev", "user,id=net0,hostfwd=tcp::2222-:22",
        "-device", "virtio-net-pci,netdev=net0",
        "-serial", "file:" .. shared.logfile,
        "-no-reboot"
    }
    if os.host() == "macosx" then
        table.join2(argv, {"-display", "cocoa,show-cursor=on,zoom-to-fit=on"})
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
    print("dotfiles  " .. (shared.dotfiles or "none") .. " -> baked into disk")
    print("log       " .. shared.logfile)
    if program and os.isfile(shared.diskfile) then
        print("run       " .. program .. " " .. table.concat(qemu_args(option, os), " "))
    end
end)
set_menu {
    usage = "xmake doctor",
    description = "Show host, tools, tarball, disk, baked dotfiles and snapshots"
}

task("vm")
on_run(function ()
    import("core.base.option")
    import("lib.detect.find_tool")
    local g = shared.guest()
    local program = qemu_program(find_tool)
    if not program then
        os.raise("qemu-system-" .. g.arch .. " is not in PATH")
    end
    if option.get("fresh") and os.isfile(shared.diskfile) then
        os.rm(shared.diskfile)
        print("dropped " .. shared.diskfile)
    end
    local started = os.time()
    local had_disk = os.isfile(shared.diskfile)
    if not option.get("dry-run") then
        if not had_disk or not os.isfile(shared.tarball) then
            shared.fetch_latest(g, os, io)
        end
        shared.prepare_guest(g, os, find_tool, io)
        if not had_disk then
            print("disk setup in " .. (os.time() - started) .. "s")
            print("first boot: the guest installs its packages for ~2 min, ly appears after that")
        end
    end
    if not os.isfile(path.join(shared.rootdir, g.kernel)) then
        os.raise("guest kernel missing, run: xmake build disk")
    end
    local argv = qemu_args(option, os)
    if option.get("dry-run") then
        print(program .. " " .. table.concat(argv, " "))
        return
    end
    print("booting " .. g.arch .. " with " .. (option.get("cpus") or "8") .. " cores, " .. (option.get("mem") or "8G") .. ", log " .. shared.logfile)
    os.execv(program, argv)
end)
set_menu {
    usage = "xmake vm [options]",
    description = "Boot the guest in QEMU",
    options = {
        {nil, "width", "kv", "2560", "Guest screen width"},
        {nil, "height", "kv", "1440", "Guest screen height"},
        {nil, "mem", "kv", "8G", "Guest RAM"},
        {nil, "cpus", "kv", "8", "Guest cores"},
        {nil, "fresh", "k", nil, "Rebuild the guest disk from cache before booting"},
        {"n", "dry-run", "k", nil, "Print the QEMU command instead of running it"}
    }
}

task("snapshot")
on_run(function ()
    import("core.base.option")
    local name = option.get("name")
    if not name then
        os.raise("usage: xmake snapshot --name=<name>")
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
        os.raise("usage: xmake restore --name=<name>")
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
