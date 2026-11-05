# Call transcript

Writes down a call everyone in it agreed to be heard in, and posts the transcript when the call ends.

It needs slim-m with add-on API 4 and the call-audio listener running (decision 0104).
It never receives audio: the server turns speech into text, and this module is sent only the text.

- An admin approves "Hear voice calls", picks the voice channels it may hear and the channel it posts in.
- A member with the `start` permission asks, in a call, for it to be turned on. Everyone in the call is asked first.
- Each segment is kept in this module's own data, for 30 days unless the admin changes it.
- When the call ends it posts the transcript in time order, a few messages at a time for a long one.
- Speakers are "Speaker 1", "Speaker 2" and so on in order of first speaking, because slim-m sends modules user ids and nothing that names them.
- If slim-m sent more lines than arrived, the transcript says how many were missed.
- A call that goes silent for ten minutes without a stop counts as ended, and is posted by the timer.

The timer runs every 15 minutes, and prunes finished transcripts older than `keep_days` once a day.
