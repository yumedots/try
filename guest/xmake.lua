local shared = shared

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
    import("core.base.option")
    if option.get("config") or option.get("user") then
        shared.request_reset(os, io, option.get("user") and "user" or "config")
        return
    end
    if os.isfile(shared.diskfile) then
        os.rm(shared.diskfile)
        print("dropped " .. shared.diskfile)
    end
    for _, kept in ipairs({shared.tarball, shared.rootdir}) do
        if os.exists(kept) then
            print("kept " .. kept)
        end
    end
    print("next: xmake run (rebuilds the disk from cache, then provisions the guest)")
end)
set_menu {
    usage = "xmake reset [--config|--user]",
    description = "Drop the guest disk (keeps tarball + tree) or only guest state",
    options = {
        {nil, "config", "k", nil, "Only relink the guest dotfiles on the next boot"},
        {nil, "user", "k", nil, "Wipe the guest home on the next boot, then relink"}
    }
}

target("disk")
set_kind("phony")
set_default(false)
on_build(function (target)
    import("lib.detect.find_tool")
    shared.prepare_guest(shared.guest(), os, find_tool, io)
end)
