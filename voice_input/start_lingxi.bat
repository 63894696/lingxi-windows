@echo off
REM Prisir LingXi launcher (ASCII filename — avoids Chinese path mojibake)
cd /d "%USERPROFILE%\voice_input"
start "" pythonw "%USERPROFILE%\voice_input\lingxi_app.py"
