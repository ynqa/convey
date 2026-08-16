on run arguments
    if (count of arguments) is not 1 then
        error "usage: iterm2.send_enter.applescript SESSION_ID"
    end if

    set targetID to item 1 of arguments

    tell application "iTerm2"
        repeat with itermWindow in windows
            repeat with itermTab in tabs of itermWindow
                repeat with itermSession in sessions of itermTab
                    if ((unique ID of itermSession) as text) is targetID then
                        write itermSession text "" newline YES
                        return targetID
                    end if
                end repeat
            end repeat
        end repeat
    end tell

    error "iTerm2 session not found: " & targetID
end run
