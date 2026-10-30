-- zapret-syndata.lua : пресет с данными в SYN-пакете для winws2 (zapret2-gui)
--
-- КАК ЭТО РАБОТАЕТ (честно, без магии):
-- Кроме общего набора ветки discovered-стратегий (см. любой соседний
-- пресет: fake по tls_client_hello/http_req, multisplit, discord_voice,
-- quic) супервизор ТОЛЬКО для этого пресета (id "syndata" в
-- build_lua_preset_args) добавляет в TCP-секции экземпляр
--   --lua-desync=syndata:blob=fake_default_tls:tcp_md5:tls_mod=rnd,rndsni,dupsid
-- без ограничения по payload, ПЕРЕД payload-экземплярами, чтобы он
-- видел SYN-пакеты соединения. Этот файл определяет все три функции:
-- fake(), multisplit() и syndata(). Без определения syndata() здесь
-- движок не нашёл бы функцию (в zapret-lib.lua её нет, она только
-- в zapret-antidpi.lua).
--
-- ЧТО ДЕЛАЕТ ТЕХНИКА:
--   syndata: настоящий SYN дропается, а вместо него уходит SYN с
--     фейковым ClientHello внутри (тот же fake_default_tls с мутацией
--     tls_mod, что и у обычного fake). DPI начинает собирать поток
--     с фейковых данных внутри SYN и теряет настоящее начало
--     соединения; сервер данные в SYN, как правило, игнорирует или
--     буферизует (поведение вида TCP Fast Open), а настоящий
--     ClientHello следом обрабатывают обычные fake/multisplit.
--   fake и multisplit: эталон без переопределений (параметры GUI
--     pos=2, seqovl=5 действуют как есть).
--
-- ЧЕСТНЫЕ ОГРАНИЧЕНИЯ:
--   - Против живого DPI НЕ ИЗМЕРЕН. Новый пресет, проверит пользователь
--     автоподбором на своей Windows-машине.
--   - SYN с данными - нестандартный пакет: отдельные сети, фаерволы
--     и серверы такие SYN режут. На такой сети пресет честно проиграет
--     автоподбор (соединения не встанут), это ожидаемая цена техники,
--     а не баг.
--   - В режиме STRICT экземпляр syndata добавляется в обе гейтовые
--     секции (discord_tcp и google_tcp), в игровых секциях GAME его
--     нет: игровые секции на всех пресетах одинаковы (fake_plain).

-- nfqws1 : "--dpi-desync=syndata"
-- Эталон из zapret-antidpi.lua без изменений. Срабатывает на SYN:
-- заменяет его на SYN с фейковым payload и дропает оригинал.
function syndata(ctx, desync)
	if desync.dis.tcp then
		if bitand(desync.dis.tcp.th_flags, TH_SYN + TH_ACK)==TH_SYN then
			local dis = deepcopy(desync.dis)
			dis.payload = blob(desync, desync.arg.blob, "\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00")
			apply_fooling(desync, dis)
			if desync.arg.tls_mod then
				local pl = tls_mod_shim(desync, dis.payload, desync.arg.tls_mod, nil)
				if pl then dis.payload = pl end
			end
			if b_debug then DLOG("syndata: "..hexdump_dlog(dis.payload)) end
			if rawsend_dissect_ipfrag(dis, desync_opts(desync)) then
				return VERDICT_DROP
			end
		else
			instance_cutoff_shim(ctx, desync) -- mission complete
		end
	else
		-- do not cutoff on related icmp
		if not desync.dis.icmp then instance_cutoff_shim(ctx, desync) end
	end
end

-- nfqws1 : "--dpi-desync=fake"
-- Эталон без изменений.
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
			rawsend_payload_segmented(desync,fake_payload)
		else
			DLOG("fake: not acting on further replay pieces")
		end
	end
end

-- nfqws1 : "--dpi-desync=multisplit"
-- Эталон без изменений: параметры GUI (pos=2, seqovl=5) не трогаем.
function multisplit(ctx, desync)
	if not desync.dis.tcp then
		-- do not cutoff on related icmp
		if not desync.dis.icmp then instance_cutoff_shim(ctx, desync) end
		return
	end
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
