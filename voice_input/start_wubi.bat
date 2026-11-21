@echo off
REM 灵犀·五笔 launcher (ASCII filename — avoids Chinese path mojibake)
cd /d "%USERPROFILE%\voice_input"
start "" pythonw "%USERPROFILE%\voice_input\lingxi_ime\frontend\pinyin\app_wubi.py"
