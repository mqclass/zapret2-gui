-- zapret-discord-light.lua : лёгкий Discord-пресет для winws2 (zapret2-gui)
--
-- КАК ЭТО РАБОТАЕТ (честно, без магии):
-- GUI (supervisor.rs, ветка discovered-стратегий) запускает winws2 так:
--   --lua-init=@zapret-lib.lua --lua-init=@zapret-discord-light.lua
--   --filter-tcp=80,443,2053,2083,2087,2096,8443
--   --payload=tls_client_hello --lua-desync=fake:blob=fake_default_tls:tcp_md5:tls_mod=rnd,rndsni,dupsid
--   --payload=http_req        --lua-desync=fake:blob=fake_default_http:tcp_md5
--   --payload=tls_client_hello,http_req --lua-desync=multisplit:pos=2:seqovl=5:seqovl_pattern=0x1603030000
-- Этот файл ОБЯЗАТЕЛЬНО определяет функции fake() и multisplit(), которые
-- вызывают фиксированные аргументы выше. Код функций взят из эталона
-- strategies/zapret-antidpi.lua, вариации - только переопределением
-- параметров внутри функций.
--
-- ЧТО МЕНЯЕТ ЭТОТ ПРЕСЕТ ОТНОСИТЕЛЬНО ЭТАЛОНА:
--   fake: сброшены тяжёлые модификаторы (tls_mod и tcp_md5) - фейк
--     отправляется как есть, один раз. Меньше CPU и меньше шансов, что
--     сервер/клиент споткнётся о модифицированный пакет.
--   multisplit: только одна точка резки (pos=2) БЕЗ перекрытия (seqovl=0).
--     Это минимальная резка, которая ещё ломает наивный SNI-анализ.
--   Цель - низкие задержки для Discord поверх TCP (шлюз, медиа по 443).
--
-- ЧЕСТНЫЕ ОГРАНИЧЕНИЯ:
--   - Против живого DPI НЕ ИЗМЕРЕН. Новый пресет, проверит пользователь
--     автоподбором на своей Windows-машине.
--   - Голос Discord по UDP и STUN этот пресет НЕ трогает: в фиксированных
--     фильтрах GUI нет UDP-секции, а lua-файл фильтры добавить не может.
--     Для настоящего голосового UDP нужен отдельный --filter-udp раздел,
--     а это правка Rust-кода (build_arguments в supervisor.rs), которой
--     здесь нет. Не называйте этот пресет "голосовым" без этой оговорки.
--   - Фейковые пакеты - только встроенные fake_default_tls/http.

-- nfqws1 : "--dpi-desync=fake"
-- Вариация эталона: лёгкий фейк без tls_mod и без tcp_md5, один раз.
function fake(ctx, desync)
	direction_cutoff_opposite(ctx, desync)
	-- discord-light: сбрасываем тяжёлые модификаторы. desync.arg
	-- принадлежит только этому экземпляру desync, другие не затрагиваются.
	-- desync_opts() в zapret-lib.lua собирает fooling именно из desync.arg,
	-- поэтому nil здесь реально убирает md5 из отправки.
	desync.arg.tls_mod = nil
	desync.arg.tcp_md5 = nil
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
-- Вариация эталона: одна точка резки без перекрытия.
function multisplit(ctx, desync)
	if not desync.dis.tcp then
		-- do not cutoff on related icmp
		if not desync.dis.icmp then instance_cutoff_shim(ctx, desync) end
		return
	end
	-- discord-light: pos=2, seqovl=0 (перекрытие выключено: в коде ниже
	-- ветка seqovl срабатывает только при tonumber(seqovl)>0).
	desync.arg.pos = "2"
	desync.arg.seqovl = "0"
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
