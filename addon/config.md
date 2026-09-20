Everything here is also under **Tools → ankiquest settings…**, which is easier to use.

- `url`: the ankiquest server, e.g. `https://anki.example.com`
- `user`: your player name
- `token`: your upload token
- `notify_rank`: say something when your place on the leaderboard changes
- `streak_hours`: warn you this many hours before your streak ends (0 turns it off)
- `period`: which leaderboard the deck list shows (`hour`, `day`, `week`, `month`, `year`, `all`)

The leaderboard appears under the deck list, below your other statistics. **Tools → ankiquest inbox…** shows what other people have finished and answers it with a cheer or your own words.

Review log rows (card id, timestamp, previous interval, time taken, review type) and deck progress (deck IDs, names, daily remaining counts and review counts) are sent to your server, never card content. Deck names and progress are available privately through your upload token.

To share daily deck completions, open **Tools → ankiquest deck notifications…**, tick the decks you want to share and the people who should hear about them. Ticking a deck ticks its subdecks. Sharing is off by default. Recipients receive at most one completion message per deck per Anki day; later learning steps still count as unfinished work.
