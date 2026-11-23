@echo off
chcp 65001 > nul
:: 65001 - UTF-8

cd /d "%~dp0"
call service.bat status_zapret
call service.bat check_updates
call service.bat load_game_filter
call service.bat load_user_lists
echo:

set "BIN=%~dp0bin\"
set "LISTS=..\lists\"
set "LUA=..\lua\"
cd /d "%BIN%"

start "zapret: %~n0" /min "%BIN%winws2.exe" --wf-tcp-empty=0 --ctrack-disable=0 ^
--wf-tcp-in=80,443,2053,2083,2087,2096,8443,%GameFilterTCP% --wf-udp-in=443,19294-19344,50000-50100,%GameFilterUDP% ^
--wf-tcp-out=80,443,2053,2083,2087,2096,8443,%GameFilterTCP% --wf-udp-out=443,19294-19344,50000-50100,%GameFilterUDP% ^

--lua-init=@"%LUA%zapret-lib.lua" ^
--lua-init=@"%LUA%zapret-antidpi.lua" ^
--lua-init=@"%LUA%zapret-auto.lua" ^
--lua-init=@"%LUA%blobs.lua" ^

--name="domain quic" ^
--filter-udp=443 ^
--hostlist="%LISTS%list-general.txt" ^
--hostlist="%LISTS%list-general-user.txt" ^
--hostlist-exclude="%LISTS%list-exclude.txt" ^
--hostlist-exclude="%LISTS%list-exclude-user.txt" ^
--ipset-exclude="%LISTS%ipset-exclude.txt" ^
--ipset-exclude="%LISTS%ipset-exclude-user.txt" ^
--payload=quic_initial ^
--lua-desync=fake:blob=quic_google:repeats=6 ^
--new ^

--name="discord voice" ^
--filter-udp=19294-19344,50000-50100 ^
--filter-l7=discord,stun ^
--payload=discord_ip_discovery,stun ^
--lua-desync=fake:blob=active_discord:repeats=6 ^
--new ^

--name="discord media" ^
--filter-tcp=2053,2083,2087,2096,8443 ^
--hostlist-domains=discord.media ^
--payload=tls_client_hello ^
--lua-desync=fake:blob=tls_google:repeats=6:tcp_seq=2:tcp_ack=-66000:tcp_ts_up ^
--new ^

--name="youtube" ^
--filter-tcp=443 ^
--hostlist="%LISTS%list-google.txt" ^
--payload=tls_client_hello ^
--lua-desync=fake:blob=tls_google:repeats=6:tcp_ts=-600000:ip_id=zero ^
--lua-desync=multidisorder:pos=2,midsld:ip_id=zero ^
--new ^

--name="domain tls" ^
--filter-tcp=80,443 ^
--hostlist="%LISTS%list-general.txt" ^
--hostlist="%LISTS%list-general-user.txt" ^
--hostlist-exclude="%LISTS%list-exclude.txt" ^
--hostlist-exclude="%LISTS%list-exclude-user.txt" ^
--ipset-exclude="%LISTS%ipset-exclude.txt" ^
--ipset-exclude="%LISTS%ipset-exclude-user.txt" ^
--payload=tls_client_hello ^
--lua-desync=fake:blob=stun:repeats=6:tcp_seq=2:tcp_ack=-66000:tcp_ts_up ^
--lua-desync=fake:blob=tls_steamchat:repeats=6:tcp_seq=2:tcp_ack=-66000:tcp_ts_up ^
--lua-desync=multisplit:pos=2 ^
--payload=http_req ^
--lua-desync=fake:blob=tls_steamchat:repeats=6:tcp_seq=2:tcp_ack=-66000:tcp_ts_up ^
--lua-desync=multisplit:pos=2 ^
--new ^

--name="ip quic" ^
--filter-udp=443 ^
--ipset="%LISTS%ipset-all.txt" ^
--hostlist-exclude="%LISTS%list-exclude.txt" ^
--hostlist-exclude="%LISTS%list-exclude-user.txt" ^
--ipset-exclude="%LISTS%ipset-exclude.txt" ^
--ipset-exclude="%LISTS%ipset-exclude-user.txt" ^
--payload=quic_initial ^
--lua-desync=fake:blob=quic_google:repeats=6 ^
--new ^

--name="ip tls" ^
--filter-tcp=80,443,8443 ^
--ipset="%LISTS%ipset-all.txt" ^
--hostlist-exclude="%LISTS%list-exclude.txt" ^
--hostlist-exclude="%LISTS%list-exclude-user.txt" ^
--ipset-exclude="%LISTS%ipset-exclude.txt" ^
--ipset-exclude="%LISTS%ipset-exclude-user.txt" ^
--payload=tls_client_hello ^
--lua-desync=fake:blob=stun:repeats=6:tcp_seq=2:tcp_ack=-66000:tcp_ts_up ^
--lua-desync=fake:blob=tls_steamchat:repeats=6:tcp_seq=2:tcp_ack=-66000:tcp_ts_up ^
--lua-desync=multisplit:pos=2 ^
--payload=http_req ^
--lua-desync=fake:blob=tls_steamchat:repeats=6:tcp_seq=2:tcp_ack=-66000:tcp_ts_up ^
--lua-desync=multisplit:pos=2 ^
--new ^

--name="gamefiltertcp" ^
--filter-tcp=%GameFilterTCP% ^
--ipset="%LISTS%ipset-all.txt" ^
--ipset-exclude="%LISTS%ipset-exclude.txt" ^
--ipset-exclude="%LISTS%ipset-exclude-user.txt" ^
--out-range=-n5 ^
--payload=tls_client_hello,unknown ^
--lua-desync=fake:blob=stun:repeats=6:tcp_seq=2:tcp_ack=-66000:tcp_ts_up:payload=tls_client_hello,unknown ^
--lua-desync=fake:blob=tls_steamchat:repeats=6:tcp_seq=2:tcp_ack=-66000:tcp_ts_up:payload=tls_client_hello,unknown ^
--payload=http_req ^
--lua-desync=fake:blob=tls_steamchat:repeats=6:tcp_seq=2:tcp_ack=-66000:tcp_ts_up ^
--new ^

--name="gamefilterudp" ^
--filter-udp=%GameFilterUDP% ^
--ipset="%LISTS%ipset-all.txt" ^
--ipset-exclude="%LISTS%ipset-exclude.txt" ^
--ipset-exclude="%LISTS%ipset-exclude-user.txt" ^
--out-range=-n3 ^
--payload=all ^
--lua-desync=fake:blob=active_game:repeats=12:payload=all
