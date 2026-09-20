"""The flat deck list read as a tree, so "Spanish::Verbs" sits under "Spanish"."""


def sort_key(deck):
    return [part.lower() for part in deck["name"].split("::")]


def ordered(decks):
    return sorted(decks, key=sort_key)


def depth(deck):
    return deck["name"].count("::")


def label(deck):
    return deck["name"].split("::")[-1]


def descendants(decks, index):
    """Every deck nested below this one; they follow it, because the list is sorted."""
    prefix = decks[index]["name"] + "::"
    found = []
    for other in range(index + 1, len(decks)):
        if not decks[other]["name"].startswith(prefix):
            break
        found.append(other)
    return found


def branch(decks, index):
    return [index] + descendants(decks, index)
