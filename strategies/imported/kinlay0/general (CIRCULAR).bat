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
--hostlist="%LISTS%list-google.txt" ^
--hostlist-exclude="%LISTS%list-exclude.txt" ^
--hostlist-exclude="%LISTS%list-exclude-user.txt" ^
--ipset-exclude="%LISTS%ipset-exclude.txt" ^
--ipset-exclude="%LISTS%ipset-exclude-user.txt" ^
--payload=quic_initial ^
--lua-desync=fake:blob=quic_rutube:repeats=6 ^
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
--in-range=-s34228 ^
--out-range=-s34228 ^
--payload=all ^
--lua-desync=circular:key=dm:fails=2:time=120:retrans=2:reset ^
--payload=tls_client_hello ^
--lua-desync=tls_client_hello_clone:blob=ch_clone:fallback=tls_google:sni_del:sni_first=www.google.com:sni_snt_new=0:strategy=1 ^
--lua-desync=fake:blob=ch_clone:repeats=6:tcp_ts=-600000:ip_id=zero:strategy=1 ^
--lua-desync=fake:blob=tls_google:tls_mod=rnd,padencap:repeats=4:badsum:ip_id=zero:strategy=1 ^
--lua-desync=multisplit:pos=sniext+1,midsld:seqovl=1:ip_id=zero:strategy=1 ^
--lua-desync=fake:blob=tls_google:tls_mod=rnd,dupsid:repeats=6:tcp_ts=-600000:ip_id=zero:strategy=2 ^
--lua-desync=multidisorder:pos=1,midsld:ip_id=zero:strategy=2 ^
--new ^

--name="youtube" ^
--filter-tcp=443 ^
--hostlist="%LISTS%list-google.txt" ^
--in-range=-s34228 ^
--out-range=-s34228 ^
--payload=all ^
--lua-desync=circular:key=yt:nld=2:fails=2:time=120:retrans=2:reset ^
--payload=tls_client_hello ^
--lua-desync=tls_client_hello_clone:blob=ch_clone:fallback=tls_steamchat:sni_del:sni_first=winelab.ru:sni_snt_new=0:strategy=1 ^
--lua-desync=fake:blob=ch_clone:repeats=6:tcp_ts=-600000:strategy=1 ^
--lua-desync=fake:blob=tls_steamchat:tls_mod=rnd,padencap:repeats=4:badsum:strategy=1 ^
--lua-desync=multisplit:pos=sniext+1,midsld:seqovl=1:strategy=1 ^
--lua-desync=fake:blob=tls_sferum:repeats=6:tcp_ts=-600000:strategy=2 ^
--lua-desync=multisplit:pos=2:ip_id=zero:strategy=2 ^
--new ^

--name="domain tls" ^
--filter-tcp=80,443 ^
--hostlist="%LISTS%list-general.txt" ^
--hostlist="%LISTS%list-general-user.txt" ^
--hostlist-exclude="%LISTS%list-exclude.txt" ^
--hostlist-exclude="%LISTS%list-exclude-user.txt" ^
--ipset-exclude="%LISTS%ipset-exclude.txt" ^
--ipset-exclude="%LISTS%ipset-exclude-user.txt" ^
--in-range=-s34228 ^
--out-range=-s34228 ^
--payload=all ^
--lua-desync=circular:key=gen:nld=2:fails=2:time=120:retrans=2:reset ^
--payload=tls_client_hello ^
--lua-desync=tls_client_hello_clone:blob=ch_clone:fallback=tls_google:sni_del:sni_first=www.google.com:sni_snt_new=0:strategy=1 ^
--lua-desync=fake:blob=ch_clone:repeats=6:tcp_ts=-600000:ip_id=zero:strategy=1 ^
--lua-desync=fake:blob=tls_google:tls_mod=rnd,padencap:repeats=4:badsum:ip_id=zero:strategy=1 ^
--lua-desync=multisplit:pos=sniext+1,midsld:seqovl=1:ip_id=zero:strategy=1 ^
--lua-desync=fake:blob=tls_google:tls_mod=rnd,dupsid:repeats=6:tcp_ts=-600000:ip_id=zero:strategy=2 ^
--lua-desync=multidisorder:pos=1,midsld:ip_id=zero:strategy=2 ^
--payload=http_req ^
--lua-desync=fake:blob=tls_steamchat:repeats=8:tcp_ts=-600000:strategy=1 ^
--lua-desync=multisplit:pos=2:strategy=1 ^
--lua-desync=hostfakesplit:host=rutube.ru:repeats=4:tcp_ts=-600000:strategy=2 ^
--new ^

--name="ip quic" ^
--filter-udp=443 ^
--ipset="%LISTS%ipset-all.txt" ^
--hostlist-exclude="%LISTS%list-exclude.txt" ^
--hostlist-exclude="%LISTS%list-exclude-user.txt" ^
--ipset-exclude="%LISTS%ipset-exclude.txt" ^
--ipset-exclude="%LISTS%ipset-exclude-user.txt" ^
--payload=quic_initial ^
--lua-desync=fake:blob=quic_rutube:repeats=6 ^
--new ^

--name="ip tls" ^
--filter-tcp=80,443,8443 ^
--ipset="%LISTS%ipset-all.txt" ^
--hostlist-exclude="%LISTS%list-exclude.txt" ^
--hostlist-exclude="%LISTS%list-exclude-user.txt" ^
--ipset-exclude="%LISTS%ipset-exclude.txt" ^
--ipset-exclude="%LISTS%ipset-exclude-user.txt" ^
--in-range=-s34228 ^
--out-range=-s34228 ^
--payload=all ^
--lua-desync=circular:key=ip:nld=2:fails=2:time=120:retrans=2:reset ^
--payload=tls_client_hello ^
--lua-desync=fake:blob=tls_sferum:repeats=6:tcp_ts=-600000:ip_id=zero:strategy=1 ^
--lua-desync=multisplit:pos=2:strategy=1 ^
--lua-desync=fake:blob=tls_sferum:repeats=8:tcp_ts=-600000:strategy=2 ^
--lua-desync=multidisorder:pos=2:seqovl=661:seqovl_pattern=tls_sferum:strategy=2 ^
--payload=http_req ^
--lua-desync=fake:blob=tls_sferum:repeats=8:tcp_ts=-600000:strategy=1 ^
--lua-desync=http_domcase:strategy=1 ^
--lua-desync=http_hostcase:spell=hoSt:strategy=1 ^
--lua-desync=multisplit:pos=method+2,host+1:strategy=1 ^
--lua-desync=fake:blob=tls_mail:repeats=6:tcp_ts=-600000:strategy=2 ^
--new ^

--name="gamefiltertcp" ^
--filter-tcp=%GameFilterTCP% ^
--ipset="%LISTS%ipset-all.txt" ^
--ipset-exclude="%LISTS%ipset-exclude.txt" ^
--ipset-exclude="%LISTS%ipset-exclude-user.txt" ^
--out-range=-n4 ^
--payload=tls_client_hello,unknown ^
--lua-desync=fake:blob=tls_4pda:repeats=6:tcp_ts=-600000:payload=tls_client_hello,unknown ^
--lua-desync=multisplit:pos=2:seqovl=284:seqovl_pattern=tls_4pda:payload=tls_client_hello,unknown ^
--new ^

--name="gamefilterudp" ^
--filter-udp=%GameFilterUDP% ^
--ipset="%LISTS%ipset-all.txt" ^
--ipset-exclude="%LISTS%ipset-exclude.txt" ^
--ipset-exclude="%LISTS%ipset-exclude-user.txt" ^
--out-range=-n3 ^
--payload=known,unknown ^
--lua-desync=fake:blob=active_game:repeats=8:payload=known,unknown
