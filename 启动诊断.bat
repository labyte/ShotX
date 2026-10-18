@echo off
chcp 65001 >nul
rem 诊断模式启动 ShotX：额外把预热捕获帧落盘到 %%TEMP%%\shotx-dump-0.png
set SHOTX_DUMP=1
start "" "%~dp0src-tauri\target\release\shotx.exe"
echo ShotX 已以诊断模式启动（托盘驻留）。
echo 请复现问题（按 F1 / 点托盘），然后：
echo   1. 若黑屏：按 Esc 应可退出覆盖层；4 秒无响应会自动恢复任务栏
echo   2. 把这两个文件发给我：
echo      %%TEMP%%\shotx-diag.log
echo      %%TEMP%%\shotx-dump-0.png
pause
