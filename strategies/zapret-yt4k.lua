-- zapret-yt4k.lua : агрессивный YouTube 4K пресет для winws2 (zapret2-gui)
--
-- КАК ЭТО РАБОТАЕТ (честно, без магии):
-- GUI (supervisor.rs, ветка discovered-стратегий) запускает winws2 так:
--   --lua-init=@zapret-lib.lua --lua-init=@zapret-yt4k.lua
--   --filter-tcp=80,443,2053,2083,2087,2096,8443
--   --payload=tls_client_hello --lua-desync=fake:blob=fake_default_tls:tcp_md5:tls_mod=rnd,rndsni,dupsid
--   --payload=http_req        --lua-desync=fake:blob=fake_default_http:tcp_md5
--   --payload=tls_client_hello,http_req --lua-desync=multisplit:pos=2:seqovl=5:seqovl_pattern=0x1603030000
--   --blob=discord_udp:@ACTIVE_DISCORD_UDP.bin --blob=quic_google:@quic_initial_www_google_com.bin
--   --new=discord_voice --filter-udp=19294-19344,50000-65535 --payload=all --out-range=n1-n4 --lua-desync=fake:blob=discord_udp:repeats=6
--   --new=quic --filter-udp=443 --payload=quic_initial --lua-desync=fake:blob=quic_google:repeats=11
-- То есть этот файл ОБЯЗАТЕЛЬНО определяет функции fake() и multisplit(),
-- которые вызывают фиксированные аргументы выше. Все вариации ниже сделаны
-- только переопределением параметров внутри этих функций, код функций взят
-- из эталона strategies/zapret-antidpi.lua (там же лежат все помощники:
-- direction_check, blob, rawsend_payload_segmented и т.д. из zapret-lib.lua).
--
-- ЧТО МЕНЯЕТ ЭТОТ ПРЕСЕТ ОТНОСИТЕЛЬНО ЭТАЛОНА:
--   fake: поддельный TLS/HTTP пакет отправляется 3 раза вместо 1.
--   multisplit: резка не в одной точке (pos=2), а в нескольких
--     (pos="2,host,midsld+1") с перекрытием seqovl=681 по паттерну
--     0x1603030000 (как у флагманского zapret2-ru-ultimate).
--     Цена: больше служебного трафика на каждое TLS-рукопожатие.
--
-- ЧЕСТНЫЕ ОГРАНИЧЕНИЯ:
--   - Против живого DPI НЕ ИЗМЕРЕН. Это новый пресет, его проверит
--     пользователь автоподбором на своей Windows-машине.
--   - Фейковые пакеты TCP - только встроенные в движок fake_default_tls /
--     fake_default_http; UDP-фейки GUI передаёт через --blob (см. выше).
--   - UDP тоже трогается: секции discord_voice и quic из GUI (тот же
--     рецепт, что у флагмана). Оговорка: цикл x3 в fake() утраивает и
--     UDP-фейки: голос - до 72 фейков всплеском в начале потока
--     (4 пакета по out-range=n1-n4 x 18, около 86 КБ), QUIC - 33 на
--     Initial; у флагмана 6 и 11. Это цена агрессивности, только
--     в начале потока.
--   - Никаких обещаний для всех провайдеров: DPI различается по сетям.

-- nfqws1 : "--dpi-desync=fake"
-- Вариация эталона: тот же код, но поддельный пакет отправляется 3 раза.
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
			-- yt4k: агрессивный вариант - 3 фейка вместо 1 (в фиксированных
			-- аргументах GUI repeats не задан, поэтому повтор задаём циклом)
			for _=1,3 do
				rawsend_payload_segmented(desync,fake_payload)
			end
		else
			DLOG("fake: not acting on further replay pieces")
		end
	end
end

-- nfqws1 : "--dpi-desync=multisplit"
-- Вариация эталона: параметры резки переопределены под YouTube 4K.
function multisplit(ctx, desync)
	if not desync.dis.tcp then
		-- do not cutoff on related icmp
		if not desync.dis.icmp then instance_cutoff_shim(ctx, desync) end
		return
	end
	-- yt4k: переопределяем фиксированные параметры CLI (pos=2, seqovl=5)
	-- на многоточечную резку с перекрытием 681. desync.arg принадлежит
	-- только этому экземпляру desync, другие экземпляры не затрагиваются.
	desync.arg.pos = "2,host,midsld+1"
	desync.arg.seqovl = "681"
	desync.arg.seqovl_pattern = "0x1603030000"
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
