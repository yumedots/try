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
end)
set_menu {
    usage = "xmake fetch",
    description = "Download the latest Arch Linux ARM tarball into build/"
}

task("guest")
on_run(function ()
    import("lib.detect.find_tool")
    shared.prepare_guest(shared.guest(), os, find_tool, io)
end)
set_menu {
    usage = "xmake guest",
    description = "Inject guest provisioning and build build/rootfs.qcow2 from the tarball"
}

task("reset")
on_run(function ()
    if os.isfile(shared.diskfile) then
        os.rm(shared.diskfile)
        print("dropped " .. shared.diskfile)
    end
    for _, kept in ipairs({shared.tarball, shared.rootdir}) do
        if os.exists(kept) then
            print("kept " .. kept)
        end
    end
    print("next: xmake vm (rebuilds the disk from cache, then provisions the guest)")
end)
set_menu {
    usage = "xmake reset",
    description = "Drop the guest disk and keep the cached tarball and tree"
}

target("disk")
set_kind("phony")
set_default(false)
on_build(function (target)
    import("lib.detect.find_tool")
    shared.prepare_guest(shared.guest(), os, find_tool, io)
end)
