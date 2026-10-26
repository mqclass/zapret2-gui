-- zapret-tspu-max.lua : жёсткий пресет для сурового ТСПУ (ценой скорости)
--
-- КАК ЭТО РАБОТАЕТ (честно, без магии):
-- GUI (supervisor.rs, ветка discovered-стратегий) запускает winws2 так:
--   --lua-init=@zapret-lib.lua --lua-init=@zapret-tspu-max.lua
--   --filter-tcp=80,443,2053,2083,2087,2096,8443
--   --payload=tls_client_hello --lua-desync=fake:blob=fake_default_tls:tcp_md5:tls_mod=rnd,rndsni,dupsid
--   --payload=http_req        --lua-desync=fake:blob=fake_default_http:tcp_md5
--   --payload=tls_client_hello,http_req --lua-desync=multisplit:pos=2:seqovl=5:seqovl_pattern=0x1603030000
-- Этот файл ОБЯЗАТЕЛЬНО определяет функции fake() и multisplit(), которые
-- вызывают фиксированные аргументы выше. Код взят из эталона
-- strategies/zapret-antidpi.lua.
--
-- ЧТО МЕНЯЕТ ЭТОТ ПРЕСЕТ ОТНОСИТЕЛЬНО ЭТАЛОНА:
--   fake: поддельный пакет отправляется 3 раза, с полным набором
--     модификаторов из CLI (tcp_md5, tls_mod=rnd,rndsni,dupsid).
--   multisplit: здесь реализована НЕ обычная резка, а multidisorder из
--     эталона - сегменты отправляются в ОБРАТНОМ порядке
--     (последний-первым) с перекрытием seqovl=150 по паттерну
--     0x1603030000, резка в точках "host,midsld+1,-10".
--     Функция названа multisplit, потому что именно её вызывает
--     фиксированный аргумент CLI; внутри - алгоритм multidisorder.
--   Цена: больше пакетов, переупорядочивание и перекрытие - заметно
--     медленнее лёгких пресетов, возможны проблемы на части серверов.
--     Это осознанный обмен скорости на стойкость.
--
-- ЧЕСТНЫЕ ОГРАНИЧЕНИЯ:
--   - Против живого DPI НЕ ИЗМЕРЕН. Новый пресет, проверит пользователь
--     автоподбором на своей Windows-машине. "Жёсткий" здесь означает
--     "самый агрессивный из четырёх", а не "гарантированно пробивает".
--   - seqovl=150 действует только если первая точка резки (host) дальше
--     150 байт от начала пакета, иначе движок молча отменяет перекрытие
--     (см. DLOG "seqovl cancelled") - переупорядочивание при этом
--     всё равно работает.
--   - Фейковые пакеты - только встроенные fake_default_tls/http.
--   - Только TCP: фиксированные фильтры GUI не содержат UDP-секции.

-- nfqws1 : "--dpi-desync=fake"
-- Вариация эталона: 3 фейка с полным набором модификаторов из CLI.
function fake(ctx, desync)
	direction_cutoff_opposite(ctx, desync)
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
			-- tspu-max: 3 фейка вместо 1
			for _=1,3 do
				rawsend_payload_segmented(desync,fake_payload)
			end
		else
			DLOG("fake: not acting on further replay pieces")
		end
	end
end

-- internal function for code deduplication. do not call directly
-- (копия из эталона zapret-antidpi.lua, без изменений)
function multidisorder_send(desync, data, seqovl, pos)
	for i=#pos,0,-1 do
		local pos_start = pos[i] or 1
		local pos_end = i<#pos and pos[i+1]-1 or #data
		local part = string.sub(data,pos_start,pos_end)
		local ovl=0
		if i==1 and seqovl and seqovl>0 then
			if seqovl>=pos[1] then
				DLOG("multidisorder: seqovl cancelled because seqovl "..(seqovl-1).." is not less than the first split pos "..(pos[1]-1))
			else
				ovl = seqovl - 1
				local pat="\x00"
				if desync.arg.seqovl_pattern then
					if desync.arg.optional and not blob_exist(desync, desync.arg.seqovl_pattern) then
						DLOG("multidisorder: blob '"..desync.arg.seqovl_pattern.."' not found. using zero pattern")
					else
						pat = blob(desync,desync.arg.seqovl_pattern)
					end
				end
				part = pattern(pat,1,ovl)..part
			end
		end
		if b_debug then DLOG("multidisorder: sending part "..(i+1).." "..(pos_start-1).."-"..(pos_end-1).." len="..#part.." seqovl="..ovl.." : "..hexdump_dlog(part)) end
		if not rawsend_payload_segmented(desync,part,pos_start-1-ovl) then
			return VERDICT_PASS
		end
	end
	return VERDICT_DROP
end

-- nfqws1 : "--dpi-desync=multidisorder", вызванная как multisplit
-- (имя функции обязано совпадать с фиксированным аргументом CLI).
-- Алгоритм - эталонный multidisorder, переопределены только параметры.
function multisplit(ctx, desync)
	if not desync.dis.tcp then
		-- do not cutoff on related icmp
		if not desync.dis.icmp then instance_cutoff_shim(ctx, desync) end
		return
	end
	-- tspu-max: многоточечная резка с переупорядочиванием и перекрытием.
	-- seqovl здесь - число (см. документацию multidisorder в эталоне:
	-- "N must be less than the first split pos"); если первая точка
	-- (host) окажется ближе 150 байт, перекрытие молча отменится,
	-- а переупорядочивание продолжит работать.
	desync.arg.pos = "host,midsld+1,-10"
	desync.arg.seqovl = "150"
	desync.arg.seqovl_pattern = "0x1603030000"
	direction_cutoff_opposite(ctx, desync)
	if desync.arg.optional and desync.arg.blob and not blob_exist(desync, desync.arg.blob) then
		DLOG("multidisorder: blob '"..desync.arg.blob.."' not found. skipped")
		return
	end
	local data = blob_or_def(desync, desync.arg.blob) or desync.reasm_data or desync.dis.payload
	if #data>0 and direction_check(desync) and payload_check(desync) then
		if replay_first(desync) then
			local spos = desync.arg.pos or "2"
			-- check debug to save CPU
			if b_debug then DLOG("multidisorder: split pos: "..spos) end
			local pos = resolve_multi_pos(data, desync.l7payload, spos)
			if b_debug then DLOG("multidisorder: resolved split pos: "..table.concat(zero_based_pos(pos)," ")) end
			delete_pos_1(pos) -- cannot split at the first byte
			if #pos>0 then
				local seqovl
				if desync.arg.seqovl then
					seqovl = resolve_pos(data, desync.l7payload, desync.arg.seqovl)
					if not seqovl then
						DLOG("multidisorder: seqovl cancelled because could not resolve marker '"..desync.arg.seqovl.."'")
					end
				end
				if multidisorder_send(desync, data, seqovl, pos)==VERDICT_PASS then
					return VERDICT_PASS
				end
				replay_drop_set(desync)
				return desync.arg.nodrop and VERDICT_PASS or VERDICT_DROP
			else
				DLOG("multidisorder: no valid split positions")
			end
		else
			DLOG("multidisorder: not acting on further replay pieces")
		end
		-- drop replayed packets if reasm was sent successfully in splitted form
		if replay_drop(desync) then
			return desync.arg.nodrop and VERDICT_PASS or VERDICT_DROP
		end
	end
end
