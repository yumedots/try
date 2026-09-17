set_project("try")
set_version("0.1.0")

includes("shared")
includes("guest")
includes("qemu")
includes("ui")

task("clean")
on_run(function ()
    import("core.base.option")
    os.rmdir(shared.builddir)
    print("deleted " .. shared.builddir)
    if option.get("cache") then
        os.execv("sh", {"-c", "chmod -R u+w " .. shared.cachedir .. " 2>/dev/null; rm -rf " .. shared.cachedir})
        print("deleted " .. shared.cachedir)
    else
        print("kept " .. shared.cachedir .. " (tarball + tree, drop with: xmake clean --cache)")
        print("kept " .. shared.statedir .. " (guest disk + settings, xmake reset rolls the disk back)")
    end
end)
set_menu {
    usage = "xmake clean [--cache]",
    description = "Delete build/, keeping the download and tree cache in .cache/",
    options = {
        {nil, "cache", "k", nil, "Also delete .cache/ (tarball + extracted tree)"}
    }
}
