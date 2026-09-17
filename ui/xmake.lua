local shared = shared

local projectdir = path.join(os.projectdir(), "ui")
local manifest = path.join(projectdir, "Cargo.toml")
local binary = path.join(projectdir, "target/debug/try-ui")

local function cargo(find_tool)
    local tool = find_tool("cargo")
    if not tool then
        os.raise("cargo is required to build ui")
    end
    return tool.program
end

local function build_env()
    if os.host() == "macosx" or os.getenv("DEVELOPER_DIR") then
        local developer_dir = os.getenv("DEVELOPER_DIR") or "/Applications/Xcode.app/Contents/Developer"
        if os.isdir(developer_dir) then
            return {DEVELOPER_DIR = developer_dir}
        end
    end
    return nil
end

local function run_env()
    local envs = build_env() or {}
    envs.TRY_PROJECT_DIR = shared.projectdir
    envs.TRY_STATE_DIR = shared.statedir
    return envs
end

local function launch(xmake_os)
    if os.host() == "macosx" then
        local command = table.concat({
            "info=$(dbus-daemon --session --fork --print-address=1 --print-pid=1 --address=unix:tmpdir=/tmp)",
            "address=$(printf '%s\\n' \"$info\" | sed -n '1p')",
            "pid=$(printf '%s\\n' \"$info\" | sed -n '2p')",
            "DBUS_SESSION_BUS_ADDRESS=\"$address\" " .. binary,
            "status=$?",
            "kill \"$pid\" 2>/dev/null || true",
            "exit $status"
        }, "\n")
        xmake_os.execv("sh", {"-c", command}, {envs = run_env()})
        return
    end
    xmake_os.execv(binary, {}, {envs = run_env()})
end

target("ui")
    set_kind("binary")
    set_default(true)
    set_filename("try-ui")
    set_targetdir(path.join(projectdir, "target/debug"))
    on_build(function (target)
        import("lib.detect.find_tool")
        os.execv(cargo(find_tool), {"build", "--manifest-path", manifest, "--bin", "try-ui"}, {curdir = projectdir, envs = build_env()})
    end)
    on_run(function (target)
        os.execv("xmake", {"build", "ui"}, {envs = build_env()})
        launch(os)
    end)

task("run")
on_run(function ()
    os.execv("xmake", {"build", "ui"}, {envs = build_env()})
    launch(os)
end)
set_menu {
    usage = "xmake run",
    description = "Open the launcher window with the guest inside it"
}
