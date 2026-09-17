function shared.guest()
    local g = shared.guests[os.arch()]
    if not g then
        os.raise("no guest image for host arch " .. os.arch() .. " yet")
    end
    return g
end

function shared.mirror_url(g, os, io)
    local stamp = path.join(shared.cachedir, "mirror.url")
    if os.isfile(stamp) then
        local kept = io.readfile(stamp):trim()
        if kept ~= "" then
            return kept
        end
    end
    print("testing mirrors for " .. g.tarball)
    local started = os.time()
    local probe = path.join(shared.cachedir, "mirror.probe")
    if os.isdir(probe) then
        os.rmdir(probe)
    end
    os.mkdir(probe)
    local script = [[
url="]] .. g.mirror .. g.tarball .. [["
hosts="]] .. probe .. [[/hosts"
speed="]] .. probe .. [[/speed"
i=0
while [ "$i" -lt 8 ]; do
    curl -sIL --max-time 10 -o /dev/null -w '%{url_effective}\n' "$url" >> "$hosts" 2>/dev/null
    i=$(( i + 1 ))
done
for one in $(sort -u "$hosts"); do
    curl -fsSL --max-time 20 -r 0-2097151 -o /dev/null -w '%{http_code} %{speed_download} %{url_effective}\n' "$one" >> "$speed" 2>/dev/null &
done
wait
]]
    local pick = "awk '$1 == 200 || $1 == 206' " .. probe .. "/speed 2>/dev/null | sort -k2 -rn | head -1 | cut -d' ' -f3- || true"
    local best = ""
    for _ = 1, 3 do
        os.execv("sh", {"-c", script})
        best = os.iorunv("sh", {"-c", pick}):trim()
        if best ~= "" then
            break
        end
    end
    os.rmdir(probe)
    if best == "" then
        best = g.mirror .. g.tarball
        print("no mirror answered, falling back to " .. best)
    else
        print("mirror " .. best .. " in " .. (os.time() - started) .. "s")
    end
    io.writefile(stamp, best .. "\n")
    return best
end

function shared.remote_md5(url, os)
    return os.iorunv("sh", {"-c", "curl -fsSL " .. url .. ".md5 2>/dev/null || true"}):split("%s")[1]
end

function shared.tree_current(os, io)
    local stamp = os.isfile(shared.rootstamp) and io.readfile(shared.rootstamp):trim() or ""
    local md5 = os.isfile(shared.md5file) and io.readfile(shared.md5file):trim() or ""
    return stamp ~= "" and stamp == md5 and os.isfile(path.join(shared.rootdir, "etc/passwd"))
end

function shared.swap_tree(os, io)
    os.execv("sh", {"-c", "rm -rf " .. shared.rootdir .. " && mv " .. shared.tree_new .. " " .. shared.rootdir})
    os.execv("chmod", {"-R", "u+rwX", shared.rootdir})
    local md5 = os.isfile(shared.md5file) and io.readfile(shared.md5file):trim() or ""
    io.writefile(shared.rootstamp, md5 .. "\n")
end

function shared.extract_tree(g, os, io)
    if shared.tree_current(os, io) then
        print("tree cached " .. shared.rootdir)
        return
    end
    local started = os.time()
    if os.isdir(shared.tree_new) then
        os.rmdir(shared.tree_new)
    end
    os.mkdir(shared.tree_new)
    local pigz = shared.pigz(os)
    local command = pigz and (pigz .. " -dc " .. shared.tarball .. " | tar -xpf - -C " .. shared.tree_new) or ("tar -xpf " .. shared.tarball .. " -C " .. shared.tree_new)
    print("extract " .. shared.tarball)
    os.execv("sh", {"-c", command .. " 2>/dev/null || true"})
    if not os.isfile(path.join(shared.tree_new, "etc/passwd")) then
        os.rmdir(shared.tree_new)
        os.raise("extract failed, the tarball is empty or broken, run: xmake fetch")
    end
    shared.swap_tree(os, io)
    print("extracted in " .. (os.time() - started) .. "s")
end

function shared.fetch_latest(g, os, io, force)
    os.mkdir(shared.cachedir)
    if os.isfile(shared.tarball) and os.filesize(shared.tarball) > 0 and not force then
        local cached = os.isfile(shared.md5file) and io.readfile(shared.md5file):trim() or "?"
        print("tarball cached " .. cached)
        return
    end
    local url = shared.mirror_url(g, os, io)
    local want = shared.remote_md5(url, os)
    if not want or want == "" then
        os.rm(path.join(shared.cachedir, "mirror.url"))
        os.raise("no md5 from " .. url)
    end
    if want and os.isfile(shared.tarball) and hash.md5(shared.tarball) == want then
        print("tarball up to date " .. want)
        shared.extract_tree(g, os, io)
        return
    end
    local started = os.time()
    print("download (8 parallel ranges) " .. url)
    local parts = path.join(shared.cachedir, "parts")
    if os.isdir(parts) then
        os.rmdir(parts)
    end
    os.mkdir(parts)
    local tail = "cat >/dev/null"
    if not shared.tree_current(os, io) then
        local pigz = shared.pigz(os)
        tail = (pigz and (pigz .. " -dc | tar -xpf - -C " .. shared.tree_new)) or ("tar -xpf - -C " .. shared.tree_new)
        tail = tail .. " 2>/dev/null"
        os.execv("sh", {"-c", "rm -rf " .. shared.tree_new .. " && mkdir -p " .. shared.tree_new})
        print("extracting while downloading into " .. shared.tree_new)
    end
    os.execv("sh", {"-c", "url=" .. url .. "; out=" .. shared.tarball .. "; parts=" .. parts .. [[
; size=$(curl -fsSLI "$url" | tr -d '
' | awk 'tolower($1)=="content-length:"{print $2}' | tail -1)
n=8
[ "${size:-0}" -gt 8000000 ] || n=1
chunk=$(( (${size:-0} + n - 1) / n ))
i=0
while [ "$i" -lt "$n" ]; do
    from=$(( i * chunk ))
    to=$(( from + chunk - 1 ))
    [ "$to" -ge "${size:-0}" ] && to=$(( ${size:-0} - 1 ))
    range="$from-$to"
    [ "$n" -eq 1 ] && range="$from-"
    ( curl -fsSL -r "$range" -o "$parts/part.$i" "$url" && : > "$parts/part.$i.done" ) || : > "$parts/failed" &
    i=$(( i + 1 ))
done
(
    i=0
    while [ "$i" -lt "$n" ]; do
        j=0
        while [ ! -e "$parts/part.$i.done" ]; do
            [ -e "$parts/failed" ] && exit 1
            [ "$j" -ge 3000 ] && exit 1
            sleep 0.2
            j=$(( j + 1 ))
        done
        cat "$parts/part.$i"
        i=$(( i + 1 ))
    done
) | tee "$out" | ]] .. tail .. [[ &
want=$(( ${size:-0} / 1024 ))
i=0
while [ "$i" -lt 900 ]; do
    got=$(du -sk "$parts" 2>/dev/null | awk '{print $1}')
    printf '\r%s%% (%s MB)' "$(( got * 100 / (want + 1) ))" "$(( got / 1024 ))" > /dev/tty 2>/dev/null || true
    [ "$got" -ge "$want" ] && break
    sleep 1
    i=$(( i + 1 ))
done
wait
printf '\r\n' > /dev/tty 2>/dev/null || true
rm -rf "$parts"
]]})
    local actual = hash.md5(shared.tarball)
    local streamed = want and actual == want
    if want and actual ~= want then
        print("parallel download incomplete, retrying in one stream")
        os.execv("curl", {"-fL", "--progress-bar", "--retry", "3", "-o", shared.tarball, url})
        actual = hash.md5(shared.tarball)
    end
    if want and actual ~= want then
        os.rm(path.join(shared.cachedir, "mirror.url"))
        os.raise("tarball md5 mismatch: mirror says " .. want .. ", got " .. actual)
    end
    io.writefile(shared.md5file, actual .. "\n")
    print("tarball ok " .. actual .. " in " .. (os.time() - started) .. "s")
    if streamed and os.isfile(path.join(shared.tree_new, "etc/passwd")) then
        shared.swap_tree(os, io)
        print("tree streamed in " .. (os.time() - started) .. "s")
    end
    shared.extract_tree(g, os, io)
end
