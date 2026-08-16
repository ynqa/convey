on run arguments
    if (count of arguments) is not 2 then
        error "usage: iterm2.send_text.applescript SESSION_ID TEXT"
    end if

    set targetID to item 1 of arguments
    set targetText to item 2 of arguments

    tell application "iTerm2"
        repeat with itermWindow in windows
            repeat with itermTab in tabs of itermWindow
                repeat with itermSession in sessions of itermTab
                    if ((unique ID of itermSession) as text) is targetID then
                        write itermSession text targetText newline NO
                        return targetID
                    end if
                end repeat
            end repeat
        end repeat
    end tell

    error "iTerm2 session not found: " & targetID
end run
