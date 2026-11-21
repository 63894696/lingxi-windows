# -*- mode: python ; coding: utf-8 -*-


a = Analysis(
    ['lingxi_app.py'],
    pathex=['_venv_voice/Lib/site-packages', '.'],
    binaries=[],
    datas=[('lingxi_config.py', '.'), ('lingxi.ico', '.'), ('icon_candidates/lingxi_leaf_mic.png', 'icon_candidates'), ('src/voice_input/asr/professional_corrector.py', 'src/voice_input/asr')],
    # sqlite3：professional_corrector 纠错层依赖；UPX 压 _sqlite3.pyd 会导致
    # 冻结后 ModuleNotFoundError: No module named 'sqlite3'
    hiddenimports=[
        'pystray._win32', 'pyautogui', 'pyperclip', 'lingxi_config',
        'onnxruntime.capi._pybind_state', 'voice_input.asr.professional_corrector',
        'sqlite3', '_sqlite3',
    ],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name='lingxi_voice',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=['_sqlite3.pyd', 'sqlite3.dll'],
    runtime_tmpdir=None,
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    icon='lingxi.ico',  # 任务栏/快捷方式：16–256 多尺寸（源图 1024 叶麦，不嵌 1024）
)
