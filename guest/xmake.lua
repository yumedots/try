local shared = shared

task("fetch")
on_run(function ()
    import("net.http.download")
    shared.fetch_latest(shared.guest(), os, download, io)
end)
set_menu {
    usage = "xmake fetch",
    description = "Download the latest Arch Linux ARM tarball into build/"
}

task("guest")
on_run(function ()
    import("lib.detect.find_tool")
    shared.prepare_guest(shared.guest(), os, find_tool)
end)
set_menu {
    usage = "xmake guest",
    description = "Inject guest provisioning and build build/rootfs.qcow2 from the tarball"
}

target("disk")
set_kind("phony")
set_default(false)
on_build(function (target)
    import("lib.detect.find_tool")
    shared.prepare_guest(shared.guest(), os, find_tool)
end)
