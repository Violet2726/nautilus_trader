#!/usr/bin/env python3

import datetime as dt
import itertools
import os
import platform
import re
import shutil
import subprocess
import sys
import sysconfig
from pathlib import Path

import numpy as np
from Cython.Build import build_ext
from Cython.Build import cythonize
from Cython.Compiler import Options
from Cython.Compiler.Version import version as cython_compiler_version
from packaging.version import Version
from setuptools import Distribution
from setuptools import Extension


# 平台常量
IS_LINUX = platform.system() == "Linux"
IS_MACOS = platform.system() == "Darwin"
IS_WINDOWS = platform.system() == "Windows"
IS_ARM64 = platform.machine() in ("arm64", "aarch64")


# 用于构建的 Rust 工具链
RUSTUP_TOOLCHAIN = os.getenv("RUSTUP_TOOLCHAIN", "stable")
# Cargo 构建模式
BUILD_MODE = os.getenv("BUILD_MODE", "release")
# 如果启用了 PROFILE_MODE 模式，则包含覆盖率和分析所需的追踪
PROFILE_MODE = bool(os.getenv("PROFILE_MODE", ""))
# 如果启用了 ANNOTATION 模式，则生成输入源文件的带注释 HTML 版本
ANNOTATION_MODE = bool(os.getenv("ANNOTATION_MODE", ""))
# 如果启用了 PARALLEL 构建，则在构建的编译阶段使用所有 CPU
PARALLEL_BUILD = os.getenv("PARALLEL_BUILD", "true").lower() == "true"
# 如果启用了 COPY_TO_SOURCE，则将构建好的 *.so/*.pyd 文件复制回源代码树
COPY_TO_SOURCE = os.getenv("COPY_TO_SOURCE", "true").lower() == "true"
# 即使在非 release 构建中也强制去除调试符号
FORCE_STRIP = os.getenv("FORCE_STRIP", "false").lower() == "true"
# 如果仅 PyO3，则不构建 C 扩展以减少编译时间
PYO3_ONLY = os.getenv("PYO3_ONLY", "").lower() != ""
# 如果是干跑（dry run），仅打印将要执行的命令
DRY_RUN = bool(os.getenv("DRY_RUN", ""))

# 精度模式配置
# https://nautilustrader.io/docs/nightly/getting_started/installation#precision-mode
HIGH_PRECISION = os.getenv("HIGH_PRECISION", "true").lower() == "true"
if IS_WINDOWS and HIGH_PRECISION:
    print(
        "警告：Windows 不支持高精度模式（128 位整数不可用）\n正在强制切换为标准精度（64 位）模式",
    )
    HIGH_PRECISION = False

if PROFILE_MODE:
    # 为了后续调试，C 源码需要与 Cython 代码位于同一目录中（而不是在单独的构建目录中）。
    BUILD_DIR = None
elif ANNOTATION_MODE:
    BUILD_DIR = "build/annotated"
else:
    BUILD_DIR = "build/optimized"

################################################################################
#  RUST BUILD
################################################################################

USE_SCCACHE = "sccache" in os.environ.get("CC", "") or "sccache" in os.environ.get("CXX", "")
if USE_SCCACHE:
    os.environ["RUSTC_WRAPPER"] = "sccache"
    os.environ["CARGO_INCREMENTAL"] = "0"

if IS_LINUX:
    # 默认使用 clang，但允许覆盖
    if "CC" not in os.environ:
        os.environ["CC"] = "sccache clang" if USE_SCCACHE else "clang"
    if "CXX" not in os.environ:
        os.environ["CXX"] = "sccache clang++" if USE_SCCACHE else "clang++"
    if "LDSHARED" not in os.environ:
        os.environ["LDSHARED"] = "clang -shared"

if IS_MACOS and IS_ARM64:
    os.environ["CFLAGS"] = f"{os.environ.get('CFLAGS', '')} -arch arm64"
    os.environ["LDFLAGS"] = f"{os.environ.get('LDFLAGS', '')} -arch arm64 -w"

if IS_LINUX and IS_ARM64:
    os.environ["CFLAGS"] = f"{os.environ.get('CFLAGS', '')} -fPIC"
    os.environ["LDFLAGS"] = f"{os.environ.get('LDFLAGS', '')} -fPIC"

    python_lib_dir = os.environ.get("PYTHON_LIB_DIR")
    python_version = ".".join(platform.python_version_tuple()[:2])  # e.g. "3.12"

    if python_lib_dir:
        print(f"Setting RUSTFLAGS to link with Python {python_version} in {python_lib_dir}")
        rustflags = f"{os.environ.get('RUSTFLAGS', '')} -C link-arg=-L{python_lib_dir} -C link-arg=-lpython{python_version}"
        os.environ["RUSTFLAGS"] = rustflags

if IS_WINDOWS:
    # 链接器错误 1181
    # https://docs.microsoft.com/en-US/cpp/error-messages/tool-errors/linker-tools-error-lnk1181?view=msvc-170&viewFallbackFrom=vs-2019
    RUST_LIB_PFX = ""
    RUST_STATIC_LIB_EXT = "lib"
    RUST_DYLIB_EXT = "dll"
elif IS_MACOS:
    RUST_LIB_PFX = "lib"
    RUST_STATIC_LIB_EXT = "a"
    RUST_DYLIB_EXT = "dylib"
else:  # Linux
    RUST_LIB_PFX = "lib"
    RUST_STATIC_LIB_EXT = "a"
    RUST_DYLIB_EXT = "so"

CARGO_TARGET_DIR = os.environ.get("CARGO_TARGET_DIR", Path.cwd() / "target")
CARGO_BUILD_TARGET = os.environ.get("CARGO_BUILD_TARGET", "")

# 确定 profile 目录名称
if BUILD_MODE == "release":
    profile_dir = "release"
elif BUILD_MODE == "debug-pyo3":
    profile_dir = "debug-pyo3"
else:
    profile_dir = "debug"

CARGO_TARGET_DIR = Path(CARGO_TARGET_DIR) / CARGO_BUILD_TARGET / profile_dir

# 包含头文件的目录
RUST_INCLUDES = ["nautilus_trader/core/includes"]
RUST_LIB_PATHS: list[Path] = [
    CARGO_TARGET_DIR / f"{RUST_LIB_PFX}nautilus_backtest.{RUST_STATIC_LIB_EXT}",
    CARGO_TARGET_DIR / f"{RUST_LIB_PFX}nautilus_common.{RUST_STATIC_LIB_EXT}",
    CARGO_TARGET_DIR / f"{RUST_LIB_PFX}nautilus_core.{RUST_STATIC_LIB_EXT}",
    CARGO_TARGET_DIR / f"{RUST_LIB_PFX}nautilus_model.{RUST_STATIC_LIB_EXT}",
    CARGO_TARGET_DIR / f"{RUST_LIB_PFX}nautilus_persistence.{RUST_STATIC_LIB_EXT}",
]
RUST_LIBS: list[str] = [str(path) for path in RUST_LIB_PATHS]


def _set_feature_flags() -> list[str]:
    feature_list = [
        "cython-compat",
        "extension-module",
        "ffi",
        "postgres",
        "python",
        "tracing-bridge",
    ]

    if HIGH_PRECISION:
        feature_list.append("high-precision")

    feature_list.sort()

    flags = ["--no-default-features", "--features", ",".join(feature_list)]

    return flags


def _build_rust_libs() -> None:
    print("Compiling Rust libraries...")

    try:
        # 使用 Cargo 构建 Rust 库
        if RUSTUP_TOOLCHAIN not in ("stable", "nightly"):
            raise ValueError(f"无效的 `RUSTUP_TOOLCHAIN` '{RUSTUP_TOOLCHAIN}'")

        needed_crates = [
            "nautilus-backtest",
            "nautilus-common",
            "nautilus-core",
            "nautilus-infrastructure",
            "nautilus-model",
            "nautilus-persistence",
            "nautilus-pyo3",
        ]

        if BUILD_MODE == "release":
            build_options = ["--release"]
            # 仅在 Linux 链接时传递 '-s'。在 macOS 上此标志已过时，
            # 并可能导致较新工具链失败。Cargo 已针对每个 profile 执行符号去除，
            # 我们在适用的地方进行后期去除。
            if IS_LINUX:
                existing_rustflags = os.environ.get("RUSTFLAGS", "")
                os.environ["RUSTFLAGS"] = f"{existing_rustflags} -C link-arg=-s"
        elif BUILD_MODE == "debug-pyo3":
            build_options = ["--profile", "debug-pyo3"]
        else:
            build_options = []

        features = _set_feature_flags()

        # 定义哪些 crate 支持哪些 feature，以避免 cargo 错误
        crate_features = {
            "nautilus-pyo3": ["cython-compat", "extension-module", "ffi", "postgres", "tracing-bridge"],
            "nautilus-common": ["extension-module", "ffi", "python", "tracing-bridge"],
            "nautilus-core": ["extension-module", "ffi", "python"],
            "nautilus-model": ["extension-module", "ffi", "python"],
            "nautilus-persistence": ["extension-module", "ffi", "python"],
            "nautilus-backtest": ["extension-module", "ffi", "python"],
            "nautilus-infrastructure": ["extension-module", "python", "postgres"],
        }

        shared_features = set(features)
        if "--features" in shared_features:
            shared_features.remove("--features")
        if "--no-default-features" in shared_features:
            shared_features.remove("--no-default-features")
        
        # 将 shared_features 展平（如果它包含逗号分隔的字符串）
        actual_features = []
        for f in shared_features:
            actual_features.extend(f.split(","))

        # 在单次 cargo 调用中构建所有需要的 crate，大大提高编译速度
        cmd = ["cargo", "build", "--lib", *build_options, "--no-default-features"]
        if RUSTUP_TOOLCHAIN == "nightly":
            cmd.insert(1, "+nightly")

        combined_features = []
        for crate in needed_crates:
            cmd.extend(["-p", crate])
            supported = crate_features.get(crate, [])
            to_enable = [f"{crate}/{f}" for f in actual_features if f in supported]
            combined_features.extend(to_enable)

        if combined_features:
            cmd.extend(["--features", ",".join(combined_features)])

        print(" ".join(cmd))
        subprocess.run(cmd, check=True)
    except subprocess.CalledProcessError as e:
        raise RuntimeError(
            f"Error running cargo: {e}",
        ) from e


################################################################################
# CYTHON 构建
################################################################################
# https://cython.readthedocs.io/en/latest/src/userguide/source_files_and_compilation.html

Options.docstrings = True  # 在模块中包含 docstrings
Options.fast_fail = True  # 在发生第一个错误时中止编译
Options.annotate = ANNOTATION_MODE  # 为每个 .pyx 创建带注释的 HTML 文件
if ANNOTATION_MODE:
    Options.annotate_coverage_xml = "coverage.xml"

CYTHON_COMPILER_DIRECTIVES = {
    "language_level": "3",
    "cdivision": True,  # 如果除法按照 C 方式进行且不检查零（提速 35%）
    "nonecheck": True,  # 在 C 扩展上插入额外的字段访问检查
    "embedsignature": True,  # 是否将签名嵌入到 docstrings 中
    "profile": PROFILE_MODE,  # 是否进行调试或分析
    "linetrace": PROFILE_MODE,  # 是否进行调试或分析
    "warn.maybe_uninitialized": True,
}

# TODO: 在我们需要 v3.0.11 进行覆盖率测试期间，暂时分离 Cython 配置
if Version(cython_compiler_version) >= Version("3.1.2"):
    Options.warning_errors = True  # 将编译器警告视为错误
    Options.extra_warnings = True
    CYTHON_COMPILER_DIRECTIVES["warn.deprecated.IF"] = False


def _build_extensions() -> list[Extension]:
    # 关于编译器警告：#warning "Using deprecated NumPy API,
    # disable it with " "#define NPY_NO_DEPRECATED_API NPY_1_7_API_VERSION"
    # https://stackoverflow.com/questions/52749662/using-deprecated-numpy-api
    # 源自 Cython 文档："目前这只是一个你可以忽略的警告。"
    define_macros: list[tuple[str, str | None]] = [
        ("NPY_NO_DEPRECATED_API", "NPY_1_7_API_VERSION"),
    ]
    if PROFILE_MODE or ANNOTATION_MODE:
        # 分析（Profiling）需要特殊的宏指令
        define_macros.append(("CYTHON_TRACE", "1"))

    extra_compile_args = []
    extra_link_args = RUST_LIBS

    if not IS_WINDOWS:
        # 抑制由 Cython 样板代码生成的警告
        extra_compile_args.append("-Wno-unreachable-code")
        if BUILD_MODE == "release":
            extra_compile_args.append("-O2")
            extra_compile_args.append("-pipe")

            if IS_LINUX:
                extra_compile_args.append("-ffunction-sections")
                extra_compile_args.append("-fdata-sections")
                extra_link_args.append("-Wl,--gc-sections")
                extra_link_args.append("-Wl,--as-needed")
                # 在 Linux 上确保非可执行堆栈，以避免当任何输入对象由于意外
                # 请求 execstack 时引发的加载程序错误。
                extra_link_args.append("-Wl,-z,noexecstack")

    if IS_WINDOWS:
        # 链接 Cython 扩展时所需的标准 Windows 系统库。
        # 请保持此列表小写并按字母顺序排序，以便于维护并避免重复。
        extra_compile_args.extend(["/MP", "/FS"])
        extra_link_args += [
            "advapi32.lib",
            "bcrypt.lib",
            "crypt32.lib",
            "iphlpapi.lib",
            "kernel32.lib",
            "ncrypt.lib",
            "netapi32.lib",
            "ntdll.lib",
            "ole32.lib",
            "oleaut32.lib",
            "pdh.lib",
            "powrprof.lib",
            "propsys.lib",
            "psapi.lib",
            "runtimeobject.lib",
            "schannel.lib",
            "secur32.lib",
            "shell32.lib",
            "user32.lib",
            "userenv.lib",
            "ws2_32.lib",
        ]

    print("Creating C extension modules...")
    print(f"define_macros={define_macros}")
    print(f"extra_compile_args={extra_compile_args}")

    return [
        Extension(
            name=str(pyx.relative_to(".")).replace(os.path.sep, ".")[:-4],
            sources=[str(pyx)],
            include_dirs=[np.get_include(), *RUST_INCLUDES],
            define_macros=define_macros,
            language="c",
            extra_link_args=extra_link_args,
            extra_compile_args=extra_compile_args,
        )
        for pyx in itertools.chain(Path("nautilus_trader").rglob("*.pyx"))
    ]


def _build_distribution(extensions: list[Extension]) -> Distribution:
    nthreads = os.cpu_count() or 1
    if IS_WINDOWS:
        nthreads = min(nthreads, 60)
    print(f"nthreads={nthreads}")

    distribution = Distribution(
        {
            "name": "nautilus_trader",
            "ext_modules": cythonize(
                module_list=extensions,
                compiler_directives=CYTHON_COMPILER_DIRECTIVES,
                nthreads=nthreads,
                build_dir=BUILD_DIR,
                gdb_debug=PROFILE_MODE,
            ),
            "zip_safe": False,
        },
    )
    return distribution


def _copy_build_dir_to_project(cmd: build_ext) -> None:
    # 将构建好的扩展复制回项目树
    for output in cmd.get_outputs():
        relative_extension = Path(output).relative_to(cmd.build_lib)
        if not Path(output).exists():
            continue

        # 在 Windows 上针对内存映射锁定的 .pyd 文件应用安全覆写变通方案
        if relative_extension.exists():
            try:
                relative_extension.unlink()
            except PermissionError:
                # 文件可能被锁定（例如被 IDE 中的 Python 进程加载）。
                # 我们无法在原位置删除或覆写它，但 Windows 允许重命名移动打开的文件。
                # 为了不污染 Git 工作区，我们将它们移动到已被 Git 忽略的 build 目录中
                import uuid
                old_dir = Path(BUILD_DIR or "build") / "old_pyds"
                old_dir.mkdir(parents=True, exist_ok=True)
                tmp_dst = old_dir / f"{relative_extension.name}.{uuid.uuid4().hex}.old"
                try:
                    relative_extension.rename(tmp_dst)
                except Exception as e:
                    print(f"警告：重命名锁定文件 {relative_extension} 失败：{e}")

        # 复制文件并设置权限
        shutil.copyfile(output, relative_extension)
        mode = relative_extension.stat().st_mode
        mode |= (mode & 0o444) >> 2
        try:
            relative_extension.chmod(mode)
        except PermissionError:
            print(f"警告：chmod {relative_extension} 失败")


    print("已将所有编译好的动态库文件复制到源目录")


def _copy_rust_dylibs_to_project() -> None:
    # https://pyo3.rs/latest/building-and-distribution#manual-builds
    ext_suffix = sysconfig.get_config_var("EXT_SUFFIX")
    src = Path(CARGO_TARGET_DIR) / f"{RUST_LIB_PFX}nautilus_pyo3.{RUST_DYLIB_EXT}"
    dst = Path("nautilus_trader/core") / f"nautilus_pyo3{ext_suffix}"

    if dst.exists():
        try:
            # 在 Windows 上，如果文件是只读的，或者我们想要清空覆写
            dst.unlink()
        except PermissionError:
            # 文件可能被锁定（例如被 IDE 中的 Python 进程加载）。
            # 同样为了不污染工作区，移动到忽略的 build 目录中
            import uuid
            old_dir = Path(BUILD_DIR or "build") / "old_pyds"
            old_dir.mkdir(parents=True, exist_ok=True)
            tmp_dst = old_dir / f"{dst.name}.{uuid.uuid4().hex}.old"
            try:
                dst.rename(tmp_dst)
            except Exception as e:
                print(f"警告：重命名锁定文件 {dst} 失败：{e}")

    shutil.copyfile(src=src, dst=dst)

    print(f"已将 {src} 复制到 {dst}")


def _get_nautilus_version() -> str:
    with open("pyproject.toml", encoding="utf-8") as f:
        pyproject_content = f.read().strip()
    if not pyproject_content:
        raise ValueError("pyproject.toml is empty or not properly formatted")

    version_match = re.search(r'version\s*=\s*"(.*?)"', pyproject_content)
    if not version_match:
        raise ValueError("Version not found in pyproject.toml")

    return version_match.group(1)


def _get_clang_version() -> str:
    try:
        compiler = os.environ.get("CC", "clang").split()[0]
        result = subprocess.run(
            [compiler, "--version"],
            check=True,
            capture_output=True,
        )
        output = (
            result.stdout.decode()
            .splitlines()[0]
            .replace("Apple ", "")
            .replace("Ubuntu ", "")
            .replace("clang version ", "")
            .replace("gcc (Ubuntu ", "")
            .replace(")", "")
        )
        return output
    except Exception:
        return "missing"


def _get_rustc_version() -> str:
    try:
        result = subprocess.run(
            ["rustc", "--version"],
            check=True,
            capture_output=True,
        )
        output = result.stdout.decode().lstrip("rustc ").strip()
        return output
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        err_msg = str(e) if isinstance(e, FileNotFoundError) else e.stderr.decode()
        raise RuntimeError(
            "您正在从源码安装，这需要安装 Rust 编译器。\n"
            "更多信息请访问 https://www.rust-lang.org/tools/install\n"
            f"运行 rustc 出错：{err_msg}",
        ) from e


def _ensure_windows_python_import_lib() -> None:
    """
    确保 Windows 上存在以 *t* 结尾的 Python 导入库。

    在某些官方 CPython Windows 构建中，导入库命名为 ``pythonXY.lib``（例如 ``python313.lib``）。
    但在构建 C 扩展时，``distutils``/``setuptools`` 可能会向 MSVC 链接器请求
    ``pythonXYt.lib`` 文件——注意多出的 *t* 后缀。
    *t* 变体历史上指的是线程安全（thread-safe）构建，但现在已不再分发。

    当该文件缺失时，链接器退出并报错：
    ``LINK : fatal error LNK1104: cannot open file 'pythonXYt.lib'``
    这会导致 Windows 上的 CI 构建中断。为了变通解决此问题，我们只需在
    扩展构建开始**之前**，创建一份现有导入库的副本并命以期望的名字。

    """
    if not IS_WINDOWS:
        return

    try:
        # The virtual environment as well as the base installation may both
        # participate in the link search path.  Attempt the fix in both
        # locations to maximise the chance of success.
        candidate_roots = {Path(sys.base_prefix), Path(sys.prefix)}

        # Example: for Python 3.13 -> '313'
        major, minor, *_ = platform.python_version_tuple()
        version_compact = f"{major}{minor}"

        for root in candidate_roots:
            libs_dir = root / "libs"
            if not libs_dir.exists():
                continue

            src = libs_dir / f"python{version_compact}.lib"
            dst = libs_dir / f"python{version_compact}t.lib"

            if src.exists() and not dst.exists():
                print(
                    f"正在创建缺失的 Windows 导入库 {dst}（从 {src} 复制）",
                )
                shutil.copyfile(src, dst)
    except Exception as e:  # pragma: no cover - 防御性处理
        # 永远不要因为这个辅助函数而导致构建失败，只需显示警告即可
        print(f"警告：创建以 *t* 结尾的 Python 导入库失败：{e}")


def _strip_unneeded_symbols() -> None:
    try:
        print("正在从二进制文件中去除无用符号...")
        total_before = 0
        total_after = 0

        for so in itertools.chain(Path("nautilus_trader").rglob("*.so")):
            size_before = so.stat().st_size
            total_before += size_before

            if IS_LINUX:
                strip_cmd = ["strip", "--strip-all", "-R", ".comment", "-R", ".note", so]
            elif IS_MACOS:
                strip_cmd = ["strip", "-x", so]
            else:
                raise RuntimeError(f"无法为平台 {platform.system()} 去除符号")
            subprocess.run(
                strip_cmd,  # type: ignore [arg-type]
                check=True,
                capture_output=True,
            )

            size_after = so.stat().st_size
            total_after += size_after

        if total_before > 0:
            reduction = (1 - total_after / total_before) * 100
            print(
                f"已去除符号的二进制文件：{total_before / 1024 / 1024:.1f}MB -> {total_after / 1024 / 1024:.1f}MB (减小了 {reduction:.1f}%)",
            )
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"去除符号时出错。\n{e}") from e


def show_rustanalyzer_settings() -> None:
    """
    Show appropriate vscode settings for the build.
    """
    import json

    # Set environment variables
    settings: dict[str, object] = {}
    for key in [
        "rust-analyzer.check.extraEnv",
        "rust-analyzer.runnables.extraEnv",
        "rust-analyzer.cargo.features",
    ]:
        settings[key] = {
            "CC": os.environ["CC"],
            "CXX": os.environ["CXX"],
            "VIRTUAL_ENV": os.environ["VIRTUAL_ENV"],
        }

    # Set features
    features = _set_feature_flags()
    if features[0] == "--all-features":
        settings["rust-analyzer.cargo.features"] = "all"
        settings["rust-analyzer.check.features"] = "all"
    else:
        settings["rust-analyzer.cargo.features"] = features[1].split(",")
        settings["rust-analyzer.check.features"] = features[1].split(",")

    print("请在 .vscode/settings.json 中设置以下 rust analyzer 配置")
    print(json.dumps(settings, indent=2))


def build() -> None:
    """
    构造扩展模块和分发。
    """
    _ensure_windows_python_import_lib()
    _build_rust_libs()
    # 允许在受限环境中跳过 Rust 动态库的复制
    if not os.getenv("SKIP_RUST_DYLIB_COPY"):
        _copy_rust_dylibs_to_project()

    if not PYO3_ONLY:
        # 创建 C 扩展对象以便提供给 cythonize()
        extensions = _build_extensions()
        distribution = _build_distribution(extensions)

        # 构建并运行命令
        print("正在编译 C 扩展模块...")
        cmd: build_ext = build_ext(distribution)
        if PARALLEL_BUILD:
            cmd.parallel = os.cpu_count()
            
            import concurrent.futures
            
            _original_build_extensions = cmd.build_extensions
            
            def build_extensions_parallel():
                nthreads = cmd.parallel if cmd.parallel else (os.cpu_count() or 1)
                if IS_WINDOWS:
                    nthreads = min(nthreads, 60)
                
                print(f"正在使用 {nthreads} 个线程并行化 MSVC 扩展编译...")
                with concurrent.futures.ThreadPoolExecutor(max_workers=nthreads) as executor:
                    futures = [executor.submit(cmd.build_extension, ext) for ext in cmd.extensions]
                    for future in concurrent.futures.as_completed(futures):
                        future.result()

            cmd.build_extensions = build_extensions_parallel

        cmd.ensure_finalized()
        cmd.run()

        if COPY_TO_SOURCE:
            # 将构建成果复制回源码树，以便开发和 wheel 打包
            _copy_build_dir_to_project(cmd)

    if (BUILD_MODE == "release" or FORCE_STRIP) and (IS_LINUX or IS_MACOS):
        # 针对 release 构建或强制要求时去除符号
        _strip_unneeded_symbols()


def print_env_var_if_exists(key: str) -> None:
    value = os.environ.get(key)
    if value is not None:
        print(f"{key}={value}")


if __name__ == "__main__":
    print("\033[36m")
    print("=====================================================================")
    print(f"Nautilus Builder {_get_nautilus_version()}")
    print("=====================================================================\033[0m")
    print(f"System: {platform.system()} {platform.machine()}")
    print(f"Compiler:  {_get_clang_version()}")
    print(f"Rust:      {_get_rustc_version()}")
    print(f"Python: {platform.python_version()} ({sys.executable})")
    print(f"Cython: {cython_compiler_version}")
    print(f"NumPy:  {np.__version__}")

    print(f"\nRUSTUP_TOOLCHAIN={RUSTUP_TOOLCHAIN}")
    print(f"BUILD_MODE={BUILD_MODE}")
    print(f"BUILD_DIR={BUILD_DIR}")
    print(f"HIGH_PRECISION={HIGH_PRECISION}")
    print(f"PROFILE_MODE={PROFILE_MODE}")
    print(f"ANNOTATION_MODE={ANNOTATION_MODE}")
    print(f"PARALLEL_BUILD={PARALLEL_BUILD}")
    print(f"COPY_TO_SOURCE={COPY_TO_SOURCE}")
    print(f"FORCE_STRIP={FORCE_STRIP}")
    print(f"PYO3_ONLY={PYO3_ONLY}")
    print_env_var_if_exists("CC")
    print_env_var_if_exists("CXX")
    print_env_var_if_exists("LDSHARED")
    print_env_var_if_exists("CFLAGS")
    print_env_var_if_exists("LDFLAGS")
    print_env_var_if_exists("LD_LIBRARY_PATH")
    print_env_var_if_exists("PYO3_PYTHON")
    print_env_var_if_exists("PYTHONHOME")
    print_env_var_if_exists("RUSTFLAGS")
    print_env_var_if_exists("DRY_RUN")
    print_env_var_if_exists("RUSTC_WRAPPER")
    print_env_var_if_exists("CARGO_INCREMENTAL")

    if DRY_RUN:
        show_rustanalyzer_settings()
    else:
        print("\n正在启动构建...")
        ts_start = dt.datetime.now(dt.UTC)
        build()
        print(f"构建耗时: {dt.datetime.now(dt.UTC) - ts_start}")

        print("\033[32m" + "构建已完成" + "\033[0m")
