import glob
import json
import math
import os
import re
import sys

# range regex code vendored from
# https://github.com/zhfreal/range-regex/blob/ccd5c5c89410ec5062bfb74e627788e7b31dd56f/range_regex/range_regex.py
# Copyright (c) 2013, Dmitry Voronin
# BSD 2-Clause "Simplified" License

def regex_for_range(min_, max_):
    """
    > regex_for_range(12, 345)
    '1[2-9]|[2-9]\d|[1-2]\d{2}|3[0-3]\d|34[0-5]'
    """
    positive_subpatterns = []
    negative_subpatterns = []

    if min_ < 0:
        min__ = 1
        if max_ < 0:
            min__ = abs(max_)
        max__ = abs(min_)

        negative_subpatterns = split_to_patterns(min__, max__)
        min_ = 0

    if max_ >= 0:
        positive_subpatterns = split_to_patterns(min_, max_)

    negative_only_subpatterns = ['-' + val for val in negative_subpatterns if val not in positive_subpatterns]
    positive_only_subpatterns = [val for val in positive_subpatterns if val not in negative_subpatterns]
    intersected_subpatterns = ['-?' + val for val in negative_subpatterns if val in positive_subpatterns]

    subpatterns = negative_only_subpatterns + intersected_subpatterns + positive_only_subpatterns
    return '|'.join(subpatterns)


def split_to_patterns(min_, max_):
    subpatterns = []

    start = min_
    for stop in split_to_ranges(min_, max_):
        subpatterns.append(range_to_pattern(start, stop))
        start = stop + 1

    return subpatterns


def split_to_ranges(min_, max_):
    stops = {max_}

    nines_count = 1
    stop = fill_by_nines(min_, nines_count)
    while min_ <= stop < max_:
        stops.add(stop)

        nines_count += 1
        stop = fill_by_nines(min_, nines_count)

    zeros_count = 1
    stop = fill_by_zeros(max_ + 1, zeros_count) - 1
    while min_ < stop <= max_:
        stops.add(stop)

        zeros_count += 1
        stop = fill_by_zeros(max_ + 1, zeros_count) - 1

    stops = list(stops)
    stops.sort()

    return stops


def fill_by_nines(integer, nines_count):
    return int(str(integer)[:-nines_count] + '9' * nines_count)


def fill_by_zeros(integer, zeros_count):
    return integer - integer % 10 ** zeros_count


def range_to_pattern(start, stop):
    pattern = ''
    any_digit_count = 0

    for start_digit, stop_digit in zip(str(start), str(stop)):
        if start_digit == stop_digit:
            pattern += start_digit
        elif start_digit != '0' or stop_digit != '9':
            pattern += '[{}-{}]'.format(start_digit, stop_digit)
        else:
            any_digit_count += 1

    if any_digit_count:
        pattern += r'\d'

    if any_digit_count > 1:
        pattern += '{{{}}}'.format(any_digit_count)

    return pattern


# Code derived from NLNOG RING looking glass code
# Copyright (c) 2022 Stichting NLNOG <stichting@nlnog.net>
# ISC License

def is_regular_community(community: str) -> bool:
    """ check if a community string matches a regular community, with optional ranges
    """
    re_community = re.compile(r"^[\w\-]+:[\w\-]+$")
    return re_community.match(community)


def is_large_community(community: str) -> bool:
    """ check if a community string matches a large community, with optional ranges
    """
    re_large = re.compile(r"^[\w\-]+:[\w\-]+:[\w\-]+$")
    return re_large.match(community)


def is_extended_community(community: str) -> bool:
    """ check if a community string is an extended community, with optional ranges
    """
    re_extended = re.compile(r"^\w+ [\w\-]+(:[\w\-]+)?$")
    return re_extended.match(community)


def get_community_type(community: str) -> str:
    """ determine the community type of a community.
    """
    if is_regular_community(community):
        return "regular"
    if is_large_community(community):
        return "large"
    if is_extended_community(community):
        return "extended"

    print(f"unknown community type for '{community}'", file=sys.stderr)
    return "unknown"

def read_communities() -> dict:
    """ Read the list of community definitions from communities/*.txt and translate them
        into a dictionary containing regexes
    """
    communitylist = {
        "regular": {},
        "large": {},
        "extended": {},
    }
    re_range = re.compile(r"(\d+)\-(\d+)")

    files = glob.glob(f"{sys.argv[1]}/*.txt")
    for filename in files:
        with open(filename, "r", encoding="utf8") as filehandle:
            asn = os.path.basename(filename).removeprefix("as").removesuffix(".txt")
            for entry in [line.strip() for line in filehandle.readlines()]:
                if entry.startswith("#") or "," not in entry:
                    continue
                (comm, desc) = entry.split(",", 1)
                if comm.startswith(f"{asn}:"):
                    desc = f"AS{asn}: {desc}"
                elif os.path.basename(filename) == f"as{asn}.txt":
                    print(f"Ignoring community from {os.path.basename(filename)}, as it doesn't start with the asn", file=sys.stderr)
                    continue
                ctype = get_community_type(comm)
                if ctype == "unknown":
                    print(f"unknown communtity format: '{comm}'", file=sys.stderr)
                    continue

                # funky notations:
                # nnn -> any number
                # x -> any digit
                # a-b -> numeric range a upto b
                comm = comm.lower()
                while "nnn" in comm:
                    comm = comm.replace("nnn", "(\d+)")
                while "x" in comm:
                    comm = comm.replace("x", "(\d)")
                while re_range.match(comm):
                    match = re_range.search(comm)
                    all, first, last = match.group(0), int(match.group(1)), int(match.group(2))
                    if first > last:
                        print(f"Bad range for as {comm}, {first} should be less than {last}", file=sys.stderr)
                        continue
                    comm = comm.replace(all, regex_for_range(first, last))
                communitylist[ctype][comm] = desc

    return communitylist

print(json.dumps(read_communities()))
