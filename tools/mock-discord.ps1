# Fake Discord IPC server on \\.\pipe\discord-ipc-<N> for manual end-to-end tests.
# Usage: powershell -File tools\mock-discord.ps1 -Index 1 -Log mock.log
param([int]$Index = 1, [string]$Log = "mock-discord.log")
Add-Type -AssemblyName System.Core
function ReadExact($s, $n) {
    $buf = New-Object byte[] $n; $off = 0
    while ($off -lt $n) { $r = $s.Read($buf, $off, $n - $off); if ($r -le 0) { return $null }; $off += $r }
    return $buf
}
function SendFrame($s, $op, $json) {
    $p = [Text.Encoding]::UTF8.GetBytes($json)
    $h = [BitConverter]::GetBytes([uint32]$op) + [BitConverter]::GetBytes([uint32]$p.Length)
    $s.Write($h, 0, 8); $s.Write($p, 0, $p.Length); $s.Flush()
}
while ($true) {
    $srv = New-Object IO.Pipes.NamedPipeServerStream("discord-ipc-$Index", [IO.Pipes.PipeDirection]::InOut, 1, [IO.Pipes.PipeTransmissionMode]::Byte)
    $srv.WaitForConnection()
    "[connected]" | Add-Content $Log
    try {
        while ($srv.IsConnected) {
            $h = ReadExact $srv 8; if ($null -eq $h) { break }
            $op = [BitConverter]::ToUInt32($h, 0); $len = [BitConverter]::ToUInt32($h, 4)
            $body = ReadExact $srv $len; if ($null -eq $body) { break }
            $txt = [Text.Encoding]::UTF8.GetString($body)
            "op=$op $txt" | Add-Content $Log
            if ($op -eq 0) { SendFrame $srv 1 '{"cmd":"DISPATCH","evt":"READY","data":{"v":1,"user":{"username":"mock"}}}' }
            elseif ($op -eq 1) { SendFrame $srv 1 '{"cmd":"SET_ACTIVITY","data":{},"evt":null,"nonce":"x"}' }
        }
    } catch { "[error] $_" | Add-Content $Log }
    "[disconnected]" | Add-Content $Log
    $srv.Dispose()
}
