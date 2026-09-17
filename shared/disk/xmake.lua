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

function shared.reset_disk(os, find_tool)
    if not os.isfile(shared.diskfile) then
        return false
    end
    local img = shared.qemu_img(os, find_tool)
    local listed = os.iorunv("sh", {"-c", img .. " snapshot -l '" .. shared.diskfile .. "' 2>/dev/null || true"})
    if listed:find(shared.resetsnapshot, 1, true) then
        os.execv(img, {"snapshot", "-a", shared.resetsnapshot, shared.diskfile})
        print("reset " .. shared.diskfile .. " to its " .. shared.resetsnapshot .. " snapshot")
    else
        os.rm(shared.diskfile)
        print("dropped " .. shared.diskfile .. ", it carries no " .. shared.resetsnapshot .. " snapshot")
    end
    return true
end

function shared.provision_key(os)
    local stamp = shared.tarball .. " " .. tostring(os.filesize(shared.tarball)) .. " " .. tostring(os.mtime(shared.tarball))
    local lines = {shared.disksize, stamp}
    local patterns = {path.join(shared.projectdir, "guest/**")}
    if shared.dotfiles then
        patterns[#patterns + 1] = path.join(shared.dotfiles, "**")
    end
    for _, pattern in ipairs(patterns) do
        for _, file in ipairs(os.files(pattern)) do
            lines[#lines + 1] = file .. " " .. tostring(os.filesize(file)) .. " " .. tostring(os.mtime(file))
        end
    end
    table.sort(lines)
    return table.concat(lines, "\n")
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
    shared.state(os)
    local key = shared.provision_key(os)
    local diskkey = os.isfile(shared.diskkeyfile) and io.readfile(shared.diskkeyfile) or ""
    if os.isfile(shared.diskfile) and diskkey == key then
        print("disk kept " .. shared.diskfile)
        return
    end
    local firstboot = shared.build_firstboot(os, nil, find_tool)
    local trydisplay = shared.build_trydisplay(os, nil, find_tool)
    shared.extract_tree(g, os, io)

    local prov = path.join(shared.rootdir, "usr/local/lib/try")
    if os.isdir(prov) then
        os.rmdir(prov)
    end
    os.mkdir(prov)
    os.cp(firstboot, path.join(prov, "firstboot"))
    os.cp(trydisplay, path.join(prov, "tryDisplay"))
    os.cp(path.join(shared.projectdir, "guest/packages.txt"), prov)
    os.cp(path.join(shared.projectdir, "guest/Xsetup"), path.join(prov, "Xsetup"))
    if not shared.dotfiles or not os.isdir(shared.dotfiles) then
        os.raise("dotfiles source missing or submodule not populated: " .. tostring(shared.dotfiles))
    end
    os.cp(path.join(shared.dotfiles, "*"), path.join(prov, "dotfiles"))
    os.execv("chmod", {"755", path.join(prov, "firstboot"), path.join(prov, "tryDisplay"), path.join(prov, "Xsetup")})
    os.cp(path.join(shared.projectdir, "guest/firstBoot.service"), path.join(shared.rootdir, "etc/systemd/system/firstBoot.service"))
    local wants = path.join(shared.rootdir, "etc/systemd/system/multi-user.target.wants")
    os.mkdir(wants)
    os.execv("ln", {"-sf", "/etc/systemd/system/firstBoot.service", path.join(wants, "firstBoot.service")})
    os.cp(path.join(shared.projectdir, "guest/tryDisplay.service"), path.join(shared.rootdir, "etc/systemd/system/tryDisplay.service"))
    local graphical_wants = path.join(shared.rootdir, "etc/systemd/system/graphical.target.wants")
    os.mkdir(graphical_wants)
    os.execv("ln", {"-sf", "/etc/systemd/system/tryDisplay.service", path.join(graphical_wants, "tryDisplay.service")})

    local mke2fs = shared.mke2fs(find_tool)
    if not mke2fs then
        os.raise("mke2fs not found, install e2fsprogs")
    end
    local qemu_img = shared.qemu_img(os, find_tool)
    local started = os.time()
    print("create " .. shared.rawfile .. " " .. shared.disksize)
    os.rm(shared.rawfile)
    os.execv(qemu_img, {"create", "-f", "raw", shared.rawfile, shared.disksize})
    os.execv(mke2fs, {"-t", "ext4", "-F", "-E", "lazy_itable_init=1,lazy_journal_init=1", "-J", "size=64", "-d", shared.rootdir, shared.rawfile})
    print("raw built in " .. (os.time() - started) .. "s")
    local convert_started = os.time()
    print("convert " .. shared.rawfile .. " -> " .. shared.diskfile)
    os.rm(shared.diskfile)
    os.execv(qemu_img, {"convert", "-O", "qcow2", "-m", "8", "-W", shared.rawfile, shared.diskfile})
    os.rm(shared.rawfile)
    print("disk ready in " .. (os.time() - convert_started) .. "s")
    os.execv(qemu_img, {"snapshot", "-c", shared.resetsnapshot, shared.diskfile})
    io.writefile(shared.diskkeyfile, key)
    print("size " .. os.iorunv("sh", {"-c", "ls -lh " .. shared.diskfile .. " | awk '{print $5}'"}):trim())
    print("guest image ready in " .. (os.time() - total) .. "s total")
end
