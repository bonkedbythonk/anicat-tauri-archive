-- Anicat's own mpv script: the half of the old Tauri `anicat_ui/main.lua`
-- that belongs inside the player.
--
-- What is NOT here, deliberately: the Tauri script answered the app over
-- HTTP on a hardcoded port (13370), and whenever something else held that
-- port every callback went to a stranger. Anicat's server is already
-- attached to this mpv over its IPC pipe, and mpv delivers every
-- `script-message` to IPC clients as a `client-message` event, so
-- next/previous, reload, sub/dub and the three setting toggles are plain
-- script-messages with no handler here. The server acts on them and writes
-- the results back into `user-data/anicat/*`.

local msg = require 'mp.msg'

local state = {
    aniskip = {},   -- segments the server fetched, from user-data
    chapters = {},  -- segments read out of the file's own chapters
    skips = {},     -- the merge of the two, what the prompt acts on
    active = nil,
    autoskip = false,
    position = 0,
}

-- Chapter titled literally "OP"/"ED" (optionally with a number or extra word
-- around it, e.g. "OP2", "OP 2", "Part OP") -- the scene-release convention,
-- and unambiguous: nothing else gets called that.
local function match_strict_skip_type(title)
    title = (title or ''):lower()
    if title == 'op' or title:find('^op%s') or title:find('^op%d') or title:find('%sop%s') or title:find('%sop%d') or title:find('%sop$') then
        return 'op'
    end
    if title == 'ed' or title:find('^ed%s') or title:find('^ed%d') or title:find('%sed%s') or title:find('%sed%d') or title:find('%sed$') then
        return 'ed'
    end
    return nil
end

-- Looser wording ("Intro", "Opening", "Ending", "Outro", "Credits") that
-- USUALLY also means the OP/ED, but not always: some BD releases chapter a
-- cold-open/recap segment as "Intro" *and* the real opening song as a
-- separate "OP" chapter right after it -- two different segments, only the
-- second of which is the opening.
local function match_loose_skip_type(title)
    title = (title or ''):lower()
    if title:find('intro') or title:find('opening') then
        return 'op'
    end
    if title:find('ending') or title:find('outro') or title:find('credits') then
        return 'ed'
    end
    return nil
end

-- A chapter's type, with the strict name winning the whole file. An "Intro"
-- chapter three entries before an "OP" chapter is a cold open, not the
-- opening: without this both got typed 'op' -- observed on a real BD release,
-- chapters "Intro" (0:00-1:51) and "OP" (1:51-3:21) -- and the prompt fired
-- at 0:00 on the cold open, which is what "it skips the wrong thing, too
-- early" reported as.
local function parse_chapters_for_skips()
    local skips = {}
    local chapters = mp.get_property_native('chapter-list')
    if not chapters or #chapters == 0 then
        return skips
    end
    local duration = mp.get_property_number('duration') or 0

    local has_strict = {}
    for _, chapter in ipairs(chapters) do
        local strict_type = match_strict_skip_type(chapter.title or '')
        if strict_type then
            has_strict[strict_type] = true
        end
    end

    for i, chapter in ipairs(chapters) do
        local title = chapter.title or ''
        local skip_type = match_strict_skip_type(title)
        if not skip_type then
            local loose_type = match_loose_skip_type(title)
            if loose_type and not has_strict[loose_type] then
                skip_type = loose_type
            end
        end
        if skip_type then
            local start_time = chapter.time or 0
            local end_time = duration
            if i < #chapters then
                end_time = chapters[i + 1].time or duration
            end
            skips[#skips + 1] = { type = skip_type, start = start_time, endt = end_time }
            msg.info(string.format('chapter skip: %s (%ds to %ds)', title, start_time, end_time))
        end
    end
    return skips
end

-- Chapter timestamps WIN over AniSkip's for the same type: a good release
-- chapters its own OP exactly, while AniSkip carries official broadcast
-- timings that can sit seconds off on any other cut of the episode.
local function merge_skips()
    local result = {}
    local from_chapters = {}
    for _, entry in ipairs(state.chapters) do
        result[#result + 1] = entry
        from_chapters[entry.type] = true
    end
    for _, entry in ipairs(state.aniskip) do
        if not from_chapters[entry.type] then
            result[#result + 1] = entry
        end
    end
    state.skips = result
    state.active = nil
end

local function active_skip(position)
    for _, entry in ipairs(state.skips) do
        if position >= entry.start and position <= entry.endt then
            return entry
        end
    end
    return nil
end

local function jump_to(time_pos)
    local duration = mp.get_property_number('duration') or 0
    if duration <= 0 then
        return
    end
    mp.set_property_number('time-pos', math.max(0, math.min(duration, time_pos)))
end

local function skip_current_segment()
    local skip = state.active
    if skip and skip.endt and skip.endt > state.position then
        jump_to(skip.endt)
        mp.osd_message('Skipped segment', 1.5)
    end
end

-- Runs off time-pos rather than a timer: a seek lands anywhere, and the
-- prompt has to appear for the segment the viewer seeked into, not the one
-- a tick later.
local function on_position(_, pos)
    if not pos then
        return
    end
    state.position = pos
    if mp.get_property_native('seeking') then
        return
    end
    local now = active_skip(pos)
    if now == state.active then
        return
    end
    state.active = now
    if not now then
        return
    end
    local label = now.type == 'ed' and 'Outro' or 'Intro'
    if state.autoskip then
        jump_to(now.endt)
        mp.osd_message('Skipping ' .. label, 1.5)
    elseif not now.notified then
        now.notified = true
        mp.osd_message(label .. ' -- Shift+S to skip', 3.0)
    end
end

local function toggle_sideways()
    -- Rotation through a video filter, not `video-rotate`: the filter chain
    -- is what the Tauri build used and what mpv can apply to any decoder
    -- output. Hardware decoding is dropped for the rotated pass because the
    -- filter needs frames in system memory.
    state.sideways = ((state.sideways or 0) + 1) % 3
    if state.sideways == 1 then
        state.saved_hwdec = mp.get_property('hwdec')
        mp.set_property('hwdec', 'no')
        mp.commandv('vf', 'set', 'sub,lavfi=[transpose=clock]')
        mp.osd_message('Sideways: 90 CW', 2.0)
    elseif state.sideways == 2 then
        mp.commandv('vf', 'set', 'sub,lavfi=[transpose=cclock]')
        mp.osd_message('Sideways: 90 CCW', 2.0)
    else
        mp.commandv('vf', 'clr', '')
        if state.saved_hwdec then
            mp.set_property('hwdec', state.saved_hwdec)
            state.saved_hwdec = nil
        end
        mp.osd_message('Sideways: Off', 2.0)
    end
end

-- The server writes these; the script only reads them. `user-data` survives
-- a `loadfile replace`, so the next episode inherits the current settings
-- without a round trip.
mp.observe_property('user-data/anicat/autoskip', 'bool', function(_, value)
    state.autoskip = value == true
end)

mp.observe_property('user-data/anicat/skip-times', 'native', function(_, value)
    state.aniskip = {}
    if type(value) == 'table' and type(value.segments) == 'table' then
        for _, entry in ipairs(value.segments) do
            local kind = entry.type or entry.kind
            if kind and entry.start and entry['end'] then
                state.aniskip[#state.aniskip + 1] = {
                    type = kind,
                    start = entry.start,
                    endt = entry['end'],
                }
            end
        end
    end
    merge_skips()
end)

mp.observe_property('chapter-list', 'native', function()
    state.chapters = parse_chapters_for_skips()
    merge_skips()
end)

mp.observe_property('time-pos', 'number', on_position)

mp.register_event('file-loaded', function()
    state.chapters = parse_chapters_for_skips()
    merge_skips()
end)

-- The skin's Skip button and Shift+S reach the same function. Forced, so a
-- viewer's own input.conf cannot shadow the one binding the OSD prompt
-- tells them to press.
mp.register_script_message('anicat-skip-intro', skip_current_segment)
mp.register_script_message('anicat-toggle-sideways', toggle_sideways)
mp.add_forced_key_binding('S', 'anicat-skip-segment', skip_current_segment)

msg.info('anicat.lua loaded')
