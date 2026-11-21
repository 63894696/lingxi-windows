@echo off
REM voice_input hotkey listener 启动 - Windows 用户友好快捷方式(2026-07-02 v1.2.3)
REM 双击即跑 hotkey listener(自动切到项目根)
REM 用法: 双击 scripts\start_hotkey.bat

chcp 65001 > nul
cd /d "%~dp0..\"
python examples\hotkey_listener.py
pause