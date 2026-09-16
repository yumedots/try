local shared = shared

target("firstboot")
set_kind("binary")
set_default(false)
set_targetdir(path.join(shared.builddir, "guest"))
set_filename("firstboot")
add_files(path.join(shared.firstbootdir, "*.go"))
on_build(function (target)
    import("lib.detect.find_tool")
    shared.build_firstboot(os, target:targetfile(), find_tool)
end)

target("trydisplay")
set_kind("binary")
set_default(false)
set_targetdir(path.join(shared.builddir, "guest"))
set_filename("tryDisplay")
add_files(path.join(shared.trydisplaydir, "*.go"))
on_build(function (target)
    import("lib.detect.find_tool")
    shared.build_trydisplay(os, target:targetfile(), find_tool)
end)

task("fetch")
on_run(function ()
    shared.fetch_latest(shared.guest(), os, io, true)
    if shared.qemu_keg(os) then
        shared.qemu_fetch(os, io, true)
    else
        print("qemu: no virgl bottle on this host, vm uses qemu-system-" .. shared.guest().arch .. " from PATH")
    end
end)
set_menu {
    usage = "xmake fetch",
    description = "Download the latest Arch Linux ARM tarball and the pinned qemu-virgl bottles"
}

task("guest")
on_run(function ()
    import("lib.detect.find_tool")
    shared.prepare_guest(shared.guest(), os, find_tool, io)
end)
set_menu {
    usage = "xmake guest",
    description = "Inject guest provisioning and build the rootfs.qcow2 disk from the tarball"
}

task("reset")
on_run(function ()
    import("lib.detect.find_tool")
    shared.reset_disk(os, find_tool)
    for _, kept in ipairs({shared.tarball, shared.rootdir, shared.settingsfile}) do
        if os.exists(kept) then
            print("kept " .. kept)
        end
    end
    print("next: xmake run (no rebuild, the disk is already back at its first-boot state)")
end)
set_menu {
    usage = "xmake reset",
    description = "Roll the guest disk back to its first-boot state and keep every cache"
}

target("disk")
set_kind("phony")
set_default(false)
on_build(function (target)
    import("lib.detect.find_tool")
    shared.prepare_guest(shared.guest(), os, find_tool, io)
end)
