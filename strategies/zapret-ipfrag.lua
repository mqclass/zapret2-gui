-- zapret-ipfrag.lua : fallback-пресет с IP-фрагментацией для winws2 (zapret2-gui)
--
-- КАК ЭТО РАБОТАЕТ (честно, без магии):
-- GUI (supervisor.rs, ветка discovered-стратегий) запускает winws2 с тем
-- же набором аргументов, что и соседние пресеты:
--   --lua-init=@zapret-lib.lua --lua-init=@zapret-ipfrag.lua
--   --payload=tls_client_hello --lua-desync=fake:blob=fake_default_tls:tcp_md5:tls_mod=rnd,rndsni,dupsid
--   --payload=http_req        --lua-desync=fake:blob=fake_default_http:tcp_md5
--   --payload=tls_client_hello,http_req --lua-desync=multisplit:pos=2:seqovl=5:seqovl_pattern=0x1603030000
--   --new=discord_voice --filter-udp=19294-19344,50000-65535 --payload=all --out-range=n1-n4 --lua-desync=fake:blob=discord_udp:repeats=6
--   --new=quic --filter-udp=443 --payload=quic_initial --lua-desync=fake:blob=quic_google:repeats=11
-- Этот файл ОБЯЗАТЕЛЬНО определяет функции fake() и multisplit(). Код
-- функций взят из эталона strategies/zapret-antidpi.lua.
--
-- ЧТО МЕНЯЕТ ЭТОТ ПРЕСЕТ ОТНОСИТЕЛЬНО ЭТАЛОНА:
--   Обе функции ставят desync.arg.ipfrag = "" до отправки. Это
--   Lua-эквивалент CLI-флага ipfrag: desync_opts() в zapret-lib.lua
--   передаёт desync.arg в rawsend_dissect_ipfrag, и каждый исходящий
--   пакет движок режет на IP-фрагменты функцией ipfrag2 (позиции по
--   умолчанию: TCP - 32, UDP - 8, и считаются они от начала
--   L4-заголовка, а не от начала payload: в первый фрагмент попадают
--   32 байта «L4-заголовок + начало payload», то есть при обычном
--   20-байтовом TCP-заголовке только первые 12 байт payload; у UDP
--   все 8 байт - это сам UDP-заголовок). DPI,
--   который не пересобирает IP-фрагменты, видит только первый кусок
--   без SNI и пропускает поток; сервер фрагменты собирает обратно.
--   Резка multisplit и фейк при этом стандартные (параметры GUI):
--   меняется только доставка пакетов, не их содержимое и не точки
--   резки. Фрагментируются и UDP-фейки голоса/QUIC: fake() у движка
--   одна на все секции пресета.
--
-- ЧЕСТНЫЕ ОГРАНИЧЕНИЯ:
--   - НЕ ДЛЯ ВСЕХ СЕТЕЙ: часть провайдеров, роутеров и фаерволов
--     IP-фрагменты дропает целиком - на такой сети пресет не заведётся
--     вообще и честно проиграет автоподбор. Это fallback на случай
--     жёсткого DPI, а не пресет на каждый день.
--   - Против живого DPI НЕ ИЗМЕРЕН. Новый пресет, проверит пользователь
--     автоподбором на своей Windows-машине.
--   - Фрагментация добавляет служебных пакетов и чуть больше работы
--     сети на каждое соединение.

-- nfqws1 : "--dpi-desync=fake"
-- Вариация эталона: перед отправкой включается IP-фрагментация.
function fake(ctx, desync)
	direction_cutoff_opposite(ctx, desync)
	-- ipfrag: пустое значение = функция ipfrag2 по умолчанию, как у
	-- CLI-флага "ipfrag" без значения. desync.arg принадлежит только
	-- этому экземпляру desync, другие экземпляры не затрагиваются.
	desync.arg.ipfrag = ""
	-- by default process only outgoing known payloads. works only for tcp and udp
	if (desync.dis.tcp or desync.dis.udp) and direction_check(desync) and payload_check(desync) then
		if replay_first(desync) then
			if not desync.arg.blob then
				error("fake: 'blob' arg required")
			end
			if desync.arg.optional and not blob_exist(desync, desync.arg.blob) then
				DLOG("fake: blob '"..desync.arg.blob.."' not found. skipped")
				return
			end
			local fake_payload = blob(desync, desync.arg.blob)
			if desync.reasm_data and desync.arg.tls_mod then
				local pl = tls_mod_shim(desync, fake_payload, desync.arg.tls_mod, desync.reasm_data)
				if pl then fake_payload = pl end
			end
			-- check debug to save CPU
			if b_debug then DLOG("fake: "..hexdump_dlog(fake_payload)) end
			rawsend_payload_segmented(desync,fake_payload)
		else
			DLOG("fake: not acting on further replay pieces")
		end
	end
end

-- nfqws1 : "--dpi-desync=multisplit"
-- Вариация эталона: каждая отрезанная часть уходит IP-фрагментами.
function multisplit(ctx, desync)
	if not desync.dis.tcp then
		-- do not cutoff on related icmp
		if not desync.dis.icmp then instance_cutoff_shim(ctx, desync) end
		return
	end
	-- ipfrag: см. комментарий в fake() выше. Точки резки и seqovl
	-- остаются параметрами GUI (pos=2, seqovl=5).
	desync.arg.ipfrag = ""
	direction_cutoff_opposite(ctx, desync)
	if desync.arg.optional and desync.arg.blob and not blob_exist(desync, desync.arg.blob) then
		DLOG("multisplit: blob '"..desync.arg.blob.."' not found. skipped")
		return
	end
	local data = blob_or_def(desync, desync.arg.blob) or desync.reasm_data or desync.dis.payload
	if #data>0 and direction_check(desync) and payload_check(desync) then
		if replay_first(desync) then
			local spos = desync.arg.pos or "2"
			-- check debug to save CPU
			if b_debug then DLOG("multisplit: split pos: "..spos) end
			local pos = resolve_multi_pos(data, desync.l7payload, spos)
			if b_debug then DLOG("multisplit: resolved split pos: "..table.concat(zero_based_pos(pos)," ")) end
			delete_pos_1(pos) -- cannot split at the first byte
			if #pos>0 then
				for i=0,#pos do
					local pos_start = pos[i] or 1
					local pos_end = i<#pos and pos[i+1]-1 or #data
					local part = string.sub(data,pos_start,pos_end)
					local seqovl=0
					if i==0 and desync.arg.seqovl and tonumber(desync.arg.seqovl)>0 then
						seqovl = tonumber(desync.arg.seqovl)
						local pat="\x00"
						if desync.arg.seqovl_pattern then
							if desync.arg.optional and not blob_exist(desync, desync.arg.seqovl_pattern) then
								DLOG("multisplit: blob '"..desync.arg.seqovl_pattern.."' not found. using zero pattern")
							else
								pat = blob(desync,desync.arg.seqovl_pattern)
							end
						end
						part = pattern(pat,1,seqovl)..part
					end
					if b_debug then DLOG("multisplit: sending part "..(i+1).." "..(pos_start-1).."-"..(pos_end-1).." len="..#part.." seqovl="..seqovl.." : "..hexdump_dlog(part)) end
					if not rawsend_payload_segmented(desync,part,pos_start-1-seqovl) then
						return VERDICT_PASS
					end
				end
				replay_drop_set(desync)
				return desync.arg.nodrop and VERDICT_PASS or VERDICT_DROP
			else
				DLOG("multisplit: no valid split positions")
			end
		else
			DLOG("multisplit: not acting on further replay pieces")
		end
		-- drop replayed packets if reasm was sent successfully in splitted form
		if replay_drop(desync) then
			return desync.arg.nodrop and VERDICT_PASS or VERDICT_DROP
		end
	end
end
