local shared = shared

local function qemu_config(option, os, io, program)
    local settings = shared.settings(os, io)
    return {
        cores = option.get("cpus") or settings.cores,
        mem = option.get("mem") or settings.mem,
        audio = settings.audio,
        bridge = option.get("bridge"),
        virtualization = shared.virtualization(os, io, program)
    }
end

local function qemu_args(option, os, config)
    local g = shared.guest()
    local mac = os.host() == "macosx"
    local accel = mac and "hvf" or "kvm"
    local gl = not option.get("no-gl") and (not config.bridge or option.get("gl"))
    local machine = "virt,accel=" .. accel .. ",highmem=on"
    if config.virtualization then
        machine = machine .. ",virtualization=on"
    end
    local argv = {
        "-machine", machine,
        "-cpu", "host",
        "-smp", config.cores,
        "-m", config.mem,
        "-kernel", path.join(shared.rootdir, g.kernel),
        "-initrd", path.join(shared.rootdir, g.initrd),
        "-append", "root=/dev/vda rw console=tty0 console=ttyAMA0",
        "-drive", "if=virtio,format=qcow2,file=" .. shared.diskfile,
        "-device", (gl and "virtio-gpu-gl-pci" or "virtio-gpu-pci") ..
            ",xres=" .. (option.get("width") or "2560") .. ",yres=" .. (option.get("height") or "1440"),
        "-device", "virtio-rng-pci",
        "-device", "qemu-xhci",
        "-device", "usb-kbd",
        "-device", "usb-tablet",
        "-netdev", "user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22",
        "-device", "virtio-net-pci,netdev=net0",
        "-serial", "file:" .. shared.logfile,
        "-no-reboot"
    }
    if config.audio == "on" then
        table.join2(argv, {"-audiodev", (mac and "coreaudio" or "pa") .. ",id=audio0",
            "-device", "intel-hda", "-device", "hda-output,audiodev=audio0"})
    end
    local sharedir = shared.qemu_sharedir(os)
    if sharedir then
        table.join2(argv, {"-L", sharedir})
    end
    if config.bridge then
        table.join2(argv, {"-display", "dbus,gl=" .. (gl and "on" or "off")})
    elseif mac then
        table.join2(argv, {"-display", gl and "cocoa,gl=es,show-cursor=on" or "cocoa,show-cursor=on,zoom-to-fit=on"})
    else
        table.join2(argv, {"-display", gl and "gtk,gl=on" or "gtk"})
    end
    return argv
end

task("doctor")
on_run(function ()
    import("core.base.option")
    import("lib.detect.find_tool")
    local g = shared.guest()
    local program = shared.qemu_program(os, find_tool)
    local mke2fs = shared.mke2fs(find_tool)
    print("host      " .. os.host() .. "/" .. os.arch())
    print("guest     " .. g.arch)
    if program then
        print("qemu      " .. program .. " " .. os.iorunv(program, {"--version"}):split("\n")[1])
    else
        print("qemu      missing qemu-system-" .. g.arch)
    end
    if os.host() == "macosx" then
        print("gpu       " .. (shared.qemu_keg(os) and "virtio-gpu-gl-pci, hvf, cocoa,gl=es, bottle in " .. shared.qemudir or
            "no virgl bottle (arm64 tahoe only), use --no-gl with a QEMU from PATH"))
    else
        print("gpu       virtio-gpu-gl-pci, kvm, gtk,gl=on, system qemu-system-" .. g.arch)
    end
    local settings = shared.settings(os, io)
    print("settings  cores " .. settings.cores .. ", mem " .. settings.mem .. ", audio " .. settings.audio)
    print("nested    " .. (shared.virtualization(os, io, program) and "on, guest gets /dev/kvm" or "off"))
    print("state     " .. shared.statedir)
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
        local snapshots = os.iorunv(shared.qemu_img(os, find_tool), {"snapshot", "-l", shared.diskfile}):trim()
        print("disk      qcow2" .. (snapshots ~= "" and "\n" .. snapshots or ""))
    else
        print("disk      missing, run xmake build disk")
    end
    print("dotfiles  " .. (shared.dotfiles or "none") .. " -> baked into disk")
    print("log       " .. shared.logfile)
    if program and os.isfile(shared.diskfile) then
        print("run       " .. program .. " " .. table.concat(qemu_args(option, os, qemu_config(option, os, io, program)), " "))
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
    local bridge = option.get("bridge")
    local printing = option.get("print-args")
    shared.state(os)
    if not printing and not bridge and not option.get("no-gl") then
        shared.qemu_fetch(os, io)
    end
    local program = shared.qemu_program(os, find_tool)
    if not program then
        os.raise("qemu-system-" .. g.arch .. " is not in PATH")
    end
    if option.get("fresh") and os.isfile(shared.diskfile) then
        os.rm(shared.diskfile)
        print("dropped " .. shared.diskfile)
    end
    if not printing and not option.get("dry-run") then
        local started = os.time()
        local had_disk = os.isfile(shared.diskfile)
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
    local config = qemu_config(option, os, io, program)
    local argv = qemu_args(option, os, config)
    if printing then
        print("program=" .. program)
        for _, value in ipairs(argv) do
            print("arg=" .. value)
        end
        for key, value in pairs(shared.qemu_env(os) or {}) do
            print("env=" .. key .. "=" .. value)
        end
        return
    end
    if option.get("dry-run") then
        print(program .. " " .. table.concat(argv, " "))
        return
    end
    print("booting " .. g.arch .. " with " .. config.cores .. " cores, " .. config.mem .. ", log " .. shared.logfile)
    os.execv(program, argv, {envs = shared.qemu_env(os) or {}})
end)
set_menu {
    usage = "xmake vm [options]",
    description = "Boot the guest: virtio-gpu-gl window, or --bridge for the GPUI display",
    options = {
        {nil, "width", "kv", "2560", "Guest screen width"},
        {nil, "height", "kv", "1440", "Guest screen height"},
        {nil, "mem", "kv", nil, "Guest RAM, defaults to the saved setting"},
        {nil, "cpus", "kv", nil, "Guest cores, defaults to the saved setting"},
        {nil, "bridge", "k", nil, "Export the display over D-Bus for the GPUI window instead of opening one"},
        {nil, "print-args", "k", nil, "Print program= and arg= lines for the launcher to run"},
        {nil, "fresh", "k", nil, "Rebuild the guest disk from cache before booting"},
        {nil, "gl", "k", nil, "With --bridge, export a GL display (virtio-gpu-gl) instead of the software one"},
        {nil, "no-gl", "k", nil, "Skip the virgl bottle and use a QEMU from PATH without virtio-gpu-gl"},
        {"n", "dry-run", "k", nil, "Print the QEMU command instead of running it"}
    }
}

task("snapshot")
on_run(function ()
    import("core.base.option")
    import("lib.detect.find_tool")
    local name = option.get("name")
    if not name then
        os.raise("usage: xmake snapshot --name=<name>")
    end
    local img = shared.qemu_img(os, find_tool)
    os.execv(img, {"snapshot", "-c", name, shared.diskfile})
    print(os.iorunv(img, {"snapshot", "-l", shared.diskfile}))
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
    import("lib.detect.find_tool")
    local name = option.get("name")
    if not name then
        os.raise("usage: xmake restore --name=<name>")
    end
    os.execv(shared.qemu_img(os, find_tool), {"snapshot", "-a", name, shared.diskfile})
    print("restored " .. name)
end)
set_menu {
    usage = "xmake restore --name=<name>",
    description = "Roll the guest disk back to a snapshot (shut the VM down first)",
    options = {
        {nil, "name", "kv", nil, "Snapshot name"}
    }
}
